# 前端纯逻辑测试：用仓库自带的 tsc 编译成 commonjs，再交 node --test 跑。
# 不引 vitest —— 只测纯函数，原生 test runner 足够，少一个依赖。
# 自动发现 tools\*.test.ts，被测源码由 import 图带出，无需在此登记。
# 用法: powershell -File app/tools/run-frontend-tests.ps1
$ErrorActionPreference = 'Stop'
$w = $PSScriptRoot | Split-Path -Parent   # app/
$out = Join-Path $w '.fe-test'
if (Test-Path $out) { Remove-Item $out -Recurse -Force }

$tests = Get-ChildItem $PSScriptRoot -Filter '*.test.ts' | ForEach-Object { "tools\$($_.Name)" }
if (-not $tests) { "没有找到 tools\*.test.ts"; exit 1 }
"测试文件: " + ($tests -join ', ')

Push-Location $w
try {
  # rootDir=app 保持 src/ 与 tools/ 相对结构，编译后 import 路径不变。
  # 在 app 下运行才能自动发现 node_modules/@types/node。
  # 参数用数组 splatting：PowerShell 传 String[] 给原生 exe 会把它们
  # 拼成**一个**带空格的参数（实测 tsc 报 "File 'a.ts b.ts' not found"）。
  $tscArgs = @(
    '--ignoreConfig', '--types', 'node',
    '--rootDir', '.', '--outDir', $out,
    '--module', 'commonjs', '--moduleResolution', 'node10', '--ignoreDeprecations', '6.0',
    '--target', 'es2020', '--esModuleInterop', '--skipLibCheck'
  ) + $tests
  .\node_modules\.bin\tsc @tscArgs 2>&1 | ForEach-Object { "  $_" }
  if ($LASTEXITCODE -ne 0) { "tsc 编译失败"; exit 1 }
  # app/package.json 是 "type":"module"，编译产物是 cjs —— 就近放一个
  # commonjs 声明覆盖父级，否则 node 把 .js 当 ESM，exports 未定义。
  Set-Content -Path (Join-Path $out 'package.json') -Value '{"type":"commonjs"}' -Encoding utf8
  "编译完成，跑测试…"
  $js = $tests | ForEach-Object { Join-Path $out ($_.Replace('\', '/') -replace '\.ts$', '.js') }
  & node --test @js 2>&1
  $code = $LASTEXITCODE
} finally {
  Pop-Location
  Remove-Item $out -Recurse -Force -EA SilentlyContinue
}
exit $code
