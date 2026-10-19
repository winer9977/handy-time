/** 重复规则的中文描述 —— 待办与日程共用一份，避免两处文案漂移 */
import type { Recurrence } from '../lib/types'

const NAME: Record<Recurrence['freq'], string> = {
  daily: '每天',
  weekly: '每周',
  monthly: '每月',
  yearly: '每年',
  workday: '工作日',
  officialWorkday: '官方工作日',
  holiday: '节假日',
  lunarMonthly: '农历每月',
  lunarYearly: '农历每年',
}

const UNIT: Record<Recurrence['freq'], string> = {
  daily: '天',
  weekly: '周',
  monthly: '月',
  yearly: '年',
  workday: '工作日',
  officialWorkday: '官方工作日',
  holiday: '节假日',
  lunarMonthly: '农历月',
  lunarYearly: '农历年',
}

export function recurrenceText(r: Recurrence | null | undefined): string | null {
  if (!r) return null
  if (r.freq === 'weekly' && r.byWeekday.length) {
    // "每周五、周日" 比 "每五、日" 读得通
    return '每周' + r.byWeekday.map((d) => '一二三四五六日'[d] ?? '?').join('、')
  }
  if (r.freq === 'monthly' && r.byMonthday.length) {
    return '每月' + r.byMonthday.map((d) => (d === -1 ? '末' : d + '日')).join('/')
  }
  return r.interval > 1 ? `每${r.interval}${UNIT[r.freq]}` : NAME[r.freq]
}
