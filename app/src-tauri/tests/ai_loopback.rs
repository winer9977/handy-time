//! AI 通路的端到端测试：对着**本机回环**的假 OpenAI 服务跑一遍真实 HTTP。
//!
//! 为什么不用真端点：请求体里是用户的待办标题（隐私数据），拿它去戳外部
//! 服务需要用户先填自己的 key，我不能替用户做这个决定。回环端口既覆盖了
//! DNS/TLS 之外的全部路径（连接、方法、头、体、状态码、JSON 解析），
//! 又保证数据一个字节都不出本机。

use std::io::{Read, Write};
use std::net::TcpListener;
use xhy_lib::ai;

/// 读到一个完整 HTTP 请求（header + 按 Content-Length 的 body）。
///
/// ureq 会把 header 和 body 分在两个 TCP 包里发，单次 read 只拿得到 header，
/// 那 body 断言就会假失败。
fn read_request(sock: &mut std::net::TcpStream) -> String {
    let mut acc = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&acc).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let head = &text[..i];
                    let len = head
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if acc.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&acc).to_string()
}

/// 起一个一次性假服务：读掉请求，回一个预设响应
fn fake_openai(status_line: &str, body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    let status = status_line.to_string();
    let payload = body.to_string();
    std::thread::spawn(move || {
        if let Ok((mut sock, _)) = l.accept() {
            let req = read_request(&mut sock);
            let _ = tx.send(req);
            let resp = format!(
                "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.flush();
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
    });
    (format!("http://127.0.0.1:{port}/v1"), rx)
}

#[test]
fn ask_sends_correct_request_and_parses_reply() {
    let (base, rx) = fake_openai(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"role":"assistant","content":"本周完成度不错"}}]}"#,
    );
    let body = ai::build_body("test-model", "完成 3 项", "本周");
    let got = ai::ask(&base, "sk-local-test", &body).expect("应成功");
    assert_eq!(got, "本周完成度不错");

    // 请求本身：路径、鉴权头、模型与正文都要在
    let req = rx.recv().expect("假服务应收到请求");
    assert!(req.starts_with("POST /v1/chat/completions HTTP/1.1"), "路径/方法错：{req}");
    assert!(req.contains("authorization: Bearer sk-local-test"), "缺鉴权头：{req}");
    assert!(req.contains("application/json"), "缺 content-type：{req}");
    assert!(req.contains("test-model"), "缺模型名：{req}");
    assert!(req.contains("完成 3 项"), "缺统计正文：{req}");
}

#[test]
fn ask_reports_http_error_without_leaking_key() {
    let (base, _rx) = fake_openai("HTTP/1.1 401 Unauthorized", r#"{"error":{"message":"bad key"}}"#);
    let err = ai::ask(&base, "sk-secret-abcdef", "{}").unwrap_err();
    assert!(err.contains("401"), "应带上状态码：{err}");
    // 错误信息会显示在 UI 上，绝不能把 key 回显出去
    assert!(!err.contains("sk-secret-abcdef"), "错误信息泄露了 key：{err}");
}

#[test]
fn ask_fails_cleanly_when_nothing_listens() {
    // 端口 1 基本不会有服务：验证连接失败被转成 Err 而不是 panic
    let err = ai::ask("http://127.0.0.1:1/v1", "sk-x", "{}");
    assert!(err.is_err(), "连不上必须报错");
}

#[test]
fn ask_rejects_non_json_body() {
    let (base, _rx) = fake_openai("HTTP/1.1 200 OK", "<html>not json</html>");
    assert!(ai::ask(&base, "sk-x", "{}").is_err(), "非 JSON 响应应报错");
}
