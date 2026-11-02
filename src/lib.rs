// SPDX-License-Identifier: MIT
//! chinese-rrule — 中文自然语言时间表达式 → RFC 5545 RRULE
//! chinese-rrule — Chinese natural-language time expressions → RFC 5545 RRULE.
//!
//! 解析"每天早上8点提醒我""下周三下午3点""三天后"这类表达式，
//! 输出 RRULE 字符串（重复规则）或下次触发时间戳（一次性）。
//! Parses expressions like "每天早上8点" or "下周三下午3点" into an
//! RRULE string (recurring) or a next-trigger timestamp (one-shot).
//!
//! 设计：规则优先 / Design: rules first.
//! 规则能覆盖的表达式在亚微秒内完成；覆盖不了的返回 `None`，
//! 由调用方决定是否用 LLM 兜底（见 [`LLM_TIME_PARSE_PROMPT`] 与
//! [`parse_llm_time_json`]）。
//! Rule-parseable expressions resolve in sub-microsecond time; anything
//! else returns `None`, and the caller decides whether to fall back to an
//! LLM (see [`LLM_TIME_PARSE_PROMPT`] and [`parse_llm_time_json`]).

use chrono::{Datelike, Local, NaiveTime, Timelike};

/// 解析结果 / Parse result
#[derive(Clone, Debug)]
pub struct TimeParseResult {
    /// RRULE 字符串 (RFC 5545)，一次性提醒为空
    /// RRULE string (RFC 5545), empty for one-shot reminders
    pub rrule: String,
    /// 下次触发时间 (epoch seconds)
    /// Next trigger timestamp (epoch seconds)
    pub next_trigger_at: i64,
    /// 是否为一次性（触发后删除）
    /// Whether this is a one-shot reminder (deleted after trigger)
    pub one_shot: bool,
    /// 解析置信度 / Parse confidence
    ///
    /// - `1.0` — 精确匹配（找到明确的时间模式 + 时间）
    /// - `0.5` — 部分匹配（找到日期但时间默认 9:00）
    /// - `0.8` — 来自 [`parse_llm_time_json`]（LLM 兜底）
    /// - `0.0` — 无匹配（不应出现，parse_time 会返回 None）
    ///
    /// - `1.0` — Exact match (explicit pattern + time found)
    /// - `0.5` — Partial match (date found but time defaulted to 9:00)
    /// - `0.8` — From [`parse_llm_time_json`] (LLM fallback)
    /// - `0.0` — No match (should not appear; parse_time returns None instead)
    pub confidence: f32,
}

/// 解析中文自然语言时间表达式 / Parse a Chinese natural-language time expression
///
/// # Examples
/// - "每天早上8点提醒我开会" → FREQ=DAILY;BYHOUR=8
/// - "每周三下午3点" → FREQ=WEEKLY;BYDAY=WE;BYHOUR=15
/// - "明天下午2点" → 一次性, +1 day at 14:00
/// - "下周一下午3点" → 一次性, next Monday at 15:00
/// - "三天后提醒我" → 一次性, +3 days at 9:00
/// - "隔天早上8点" → FREQ=DAILY;INTERVAL=2;BYHOUR=8
/// - "工作日每天早上9点" → FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;BYHOUR=9
///
/// # 返回值 / Returns
/// - `Some(TimeParseResult)` — 解析成功（confidence >= 0.5）
/// - `None` — 无任何时间线索，可尝试 LLM 兜底
/// - `None` — No time clues at all; try the LLM fallback
pub fn parse_time(msg: &str) -> Option<TimeParseResult> {
    let now = Local::now();

    // 尝试提取小时:分钟，失败则用默认时间 9:00
    // Try to extract hour:minute, fall back to default 9:00
    let time_opt = extract_time(msg);
    let time = time_opt.unwrap_or(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    // 时间置信度：精确提取=1.0，默认9:00=0.5
    // Time confidence: exact extraction=1.0, default 9:00=0.5
    let time_conf = if time_opt.is_some() { 1.0f32 } else { 0.5f32 };

    // ── 重复模式 / Recurring patterns ──

    // 工作日 / Workdays — FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR
    // 必须在"每天"之前检查，否则"工作日每天"会误匹配"每天" / Must check before "每天" to avoid false match
    if msg.contains("工作日") {
        let rrule = format!(
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;BYHOUR={};BYMINUTE={}",
            time.hour(),
            time.minute()
        );
        // 计算下一个工作日 / Compute next workday
        let next = next_workday(now, time);
        return Some(TimeParseResult {
            rrule,
            next_trigger_at: next,
            one_shot: false,
            confidence: time_conf,
        });
    }

    if msg.contains("每天") || msg.contains("每日") || msg.contains("天天") {
        let rrule = format!(
            "FREQ=DAILY;BYHOUR={};BYMINUTE={}",
            time.hour(),
            time.minute()
        );
        let next = next_occurrence_daily(now, time);
        return Some(TimeParseResult {
            rrule,
            next_trigger_at: next,
            one_shot: false,
            confidence: time_conf,
        });
    }

    // 隔天 / Every other day — FREQ=DAILY;INTERVAL=2
    if msg.contains("隔天") || msg.contains("隔日") {
        let rrule = format!(
            "FREQ=DAILY;INTERVAL=2;BYHOUR={};BYMINUTE={}",
            time.hour(),
            time.minute()
        );
        let next = next_occurrence_daily(now, time);
        return Some(TimeParseResult {
            rrule,
            next_trigger_at: next,
            one_shot: false,
            confidence: time_conf,
        });
    }

    if msg.contains("每周") || msg.contains("每个星期") {
        let weekday = extract_weekday(msg)?;
        let rrule = format!(
            "FREQ=WEEKLY;BYDAY={};BYHOUR={};BYMINUTE={}",
            weekday,
            time.hour(),
            time.minute()
        );
        let next = next_occurrence_weekly(now, weekday, time);
        return Some(TimeParseResult {
            rrule,
            next_trigger_at: next,
            one_shot: false,
            confidence: time_conf,
        });
    }

    if msg.contains("每月") || msg.contains("每个月") {
        let day = extract_month_day(msg).unwrap_or(now.day());
        let rrule = format!(
            "FREQ=MONTHLY;BYMONTHDAY={};BYHOUR={};BYMINUTE={}",
            day,
            time.hour(),
            time.minute()
        );
        let next = next_occurrence_monthly(now, day, time);
        return Some(TimeParseResult {
            rrule,
            next_trigger_at: next,
            one_shot: false,
            confidence: time_conf,
        });
    }

    // ── 一次性模式 / One-shot patterns ──

    // 下周X / Next week X — 一次性
    if msg.contains("下周") {
        let weekday = extract_weekday(msg)?;
        let next = next_week_weekday(now, weekday, time);
        return Some(TimeParseResult {
            rrule: String::new(),
            next_trigger_at: next,
            one_shot: true,
            confidence: time_conf,
        });
    }

    // 这周X / This week X — 一次性
    if msg.contains("这周") || msg.contains("本周") {
        let weekday = extract_weekday(msg)?;
        let next = this_week_weekday(now, weekday, time);
        return Some(TimeParseResult {
            rrule: String::new(),
            next_trigger_at: next,
            one_shot: true,
            confidence: time_conf,
        });
    }

    // N天后 / N days later — 一次性
    if let Some(days) = parse_days_later(msg) {
        let target = now.date_naive() + chrono::Duration::days(days as i64);
        let dt = target
            .and_time(time)
            .and_local_timezone(Local)
            .single()
            .map(|d| d.timestamp())
            .unwrap_or_else(|| now.timestamp());
        return Some(TimeParseResult {
            rrule: String::new(),
            next_trigger_at: dt,
            one_shot: true,
            confidence: time_conf,
        });
    }

    // 月底 / End of month — 一次性
    if msg.contains("月底") {
        let last_day = days_in_month(now.year(), now.month());
        let next = next_occurrence_monthly(now, last_day, time);
        return Some(TimeParseResult {
            rrule: String::new(),
            next_trigger_at: next,
            one_shot: true,
            confidence: time_conf,
        });
    }

    // 下月X号 / Next month day X — 一次性
    if msg.contains("下月") {
        let day = extract_day_after_keyword(msg, "下月").unwrap_or(1);
        let next = next_month_day(now, day, time);
        return Some(TimeParseResult {
            rrule: String::new(),
            next_trigger_at: next,
            one_shot: true,
            confidence: time_conf,
        });
    }

    // 明天/后天/大后天 / Tomorrow/day after/etc.
    let days_offset = if msg.contains("明天") {
        Some(1)
    } else if msg.contains("后天") {
        Some(2)
    } else if msg.contains("大后天") {
        Some(3)
    } else {
        None
    };

    if let Some(offset) = days_offset {
        let target = now.date_naive() + chrono::Duration::days(offset);
        let dt = target
            .and_time(time)
            .and_local_timezone(Local)
            .single()
            .map(|d| d.timestamp())
            .unwrap_or_else(|| now.timestamp());
        return Some(TimeParseResult {
            rrule: String::new(),
            next_trigger_at: dt,
            one_shot: true,
            confidence: time_conf,
        });
    }

    // 裸周X（无"每"前缀）/ Bare weekday (no "每" prefix) — 一次性，最近周X
    if let Some(weekday) = extract_weekday(msg) {
        // 确保有周X模式但不是"每周" / Ensure weekday exists but not "每周"
        if has_bare_weekday(msg) {
            let next = next_occurrence_weekly(now, weekday, time);
            return Some(TimeParseResult {
                rrule: String::new(),
                next_trigger_at: next,
                one_shot: true,
                confidence: time_conf,
            });
        }
    }

    // 无任何时间线索 → None（可触发 LLM 兜底）
    // No time clues at all → None (LLM fallback is up to the caller)
    None
}

fn extract_time(msg: &str) -> Option<NaiveTime> {
    // 1. Try "H:MM" or "H：MM" format
    // 取首个匹配即可 / First match suffices
    if let Some(cap) = find_time_patterns(msg).into_iter().next() {
        let mut hour = cap.hour;
        let minute = cap.minute;
        hour = apply_period_offset(msg, hour);
        return NaiveTime::from_hms_opt(hour, minute, 0);
    }

    // 2. Try "N点MM分" / "N点MM" / "N点" format
    if let Some((hour, minute)) = extract_hour_minute(msg) {
        let h = apply_period_offset(msg, hour);
        return NaiveTime::from_hms_opt(h, minute, 0);
    }

    None
}

fn apply_period_offset(msg: &str, hour: u32) -> u32 {
    let mut h = hour;
    if msg.contains("下午") || msg.contains("晚上") || msg.contains("傍晚") {
        if h < 12 {
            h += 12;
        }
    } else if (msg.contains("凌晨") || msg.contains("深夜")) && h == 12 {
        h = 0;
    }
    if h >= 24 {
        h = 23;
    }
    h
}

struct TimeCap {
    hour: u32,
    minute: u32,
}

fn find_time_patterns(msg: &str) -> Vec<TimeCap> {
    let mut results = Vec::new();
    let chars: Vec<char> = msg.chars().collect();
    let len = chars.len();

    let mut i = 0;
    while i < len {
        if chars[i].is_ascii_digit() || is_chinese_digit(chars[i]) {
            let mut j = i;
            while j < len && (chars[j].is_ascii_digit() || is_chinese_digit(chars[j])) {
                j += 1;
            }
            let hour_str: String = chars[i..j].iter().collect();
            let hour = parse_digit(&hour_str);
            if let Some(h) = hour {
                if j + 2 < len
                    && (chars[j] == ':' || chars[j] == '：')
                    && chars[j + 1].is_ascii_digit()
                {
                    let mut k = j + 1;
                    while k < len && chars[k].is_ascii_digit() && k - (j + 1) < 2 {
                        k += 1;
                    }
                    let min_str: String = chars[j + 1..k].iter().collect();
                    if let Ok(m) = min_str.parse::<u32>() {
                        results.push(TimeCap { hour: h, minute: m });
                    }
                    i = k;
                    continue;
                }
            }
        }
        i += 1;
    }
    results
}

fn extract_hour_minute(msg: &str) -> Option<(u32, u32)> {
    let chars: Vec<char> = msg.chars().collect();
    for i in 0..chars.len() {
        if i + 1 >= chars.len() || chars[i + 1] != '点' {
            // "十二点": chars[i]=='二' && chars[i+1]=='点' && i>0 && chars[i-1]=='十'
            if chars[i] == '二'
                && i + 1 < chars.len()
                && chars[i + 1] == '点'
                && i > 0
                && chars[i - 1] == '十'
            {
                let minute = extract_minute_after_point(&chars, i + 1);
                return Some((12, minute));
            }
            continue;
        }

        // chars[i] is the char before '点'
        if let Some(h) = parse_digit(&chars[i].to_string()) {
            let minute = extract_minute_after_point(&chars, i + 1);
            return Some((h, minute));
        }
    }
    None
}

/// Extract minutes after "点" in a char slice — "点20分" "点20" "点半" → 20, 30, 0
fn extract_minute_after_point(chars: &[char], point_idx: usize) -> u32 {
    // chars[point_idx] is '点', check chars after it
    if point_idx + 1 >= chars.len() {
        return 0;
    }

    // "点半" → 30
    if chars[point_idx + 1] == '半' {
        return 30;
    }

    // Collect digits after "点"
    let mut digits = String::new();
    for &c in chars[point_idx + 1..].iter() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if !digits.is_empty() {
            break;
        }
    }

    if let Ok(m) = digits.parse::<u32>() {
        if m < 60 {
            m
        } else {
            0
        }
    } else {
        0
    }
}

fn is_chinese_digit(c: char) -> bool {
    matches!(
        c,
        '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十'
    )
}

fn parse_digit(s: &str) -> Option<u32> {
    if let Ok(n) = s.trim().parse::<u32>() {
        if n <= 9 {
            return Some(n);
        }
    }
    match s.trim() {
        "零" => Some(0),
        "一" => Some(1),
        "二" | "两" => Some(2),
        "三" => Some(3),
        "四" => Some(4),
        "五" => Some(5),
        "六" => Some(6),
        "七" => Some(7),
        "八" => Some(8),
        "九" => Some(9),
        "十" => Some(10),
        _ => None,
    }
}

fn extract_weekday(msg: &str) -> Option<&'static str> {
    let pairs = [
        ("周一", "MO"),
        ("星期一", "MO"),
        ("周二", "TU"),
        ("星期二", "TU"),
        ("周三", "WE"),
        ("星期三", "WE"),
        ("周四", "TH"),
        ("星期四", "TH"),
        ("周五", "FR"),
        ("星期五", "FR"),
        ("周六", "SA"),
        ("星期六", "SA"),
        ("周日", "SU"),
        ("星期天", "SU"),
        ("星期日", "SU"),
    ];
    for (keyword, rrule_day) in &pairs {
        if msg.contains(keyword) {
            return Some(rrule_day);
        }
    }
    None
}

fn extract_month_day(msg: &str) -> Option<u32> {
    // "每月1号" "每个月15日"
    for prefix in &["每月", "每个月"] {
        if let Some(pos) = msg.find(prefix) {
            let rest = &msg[pos + prefix.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(d) = digits.parse::<u32>() {
                if (1..=31).contains(&d) {
                    return Some(d);
                }
            }
        }
    }
    None
}

fn next_occurrence_daily(now: chrono::DateTime<Local>, time: NaiveTime) -> i64 {
    let today = now.date_naive().and_time(time);
    let today_ts = today
        .and_local_timezone(Local)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0);
    if today_ts > now.timestamp() {
        today_ts
    } else {
        (now + chrono::Duration::days(1))
            .date_naive()
            .and_time(time)
            .and_local_timezone(Local)
            .single()
            .map(|d| d.timestamp())
            .unwrap_or(0)
    }
}

fn next_occurrence_weekly(now: chrono::DateTime<Local>, target_day: &str, time: NaiveTime) -> i64 {
    let target_num = match target_day {
        "MO" => chrono::Weekday::Mon,
        "TU" => chrono::Weekday::Tue,
        "WE" => chrono::Weekday::Wed,
        "TH" => chrono::Weekday::Thu,
        "FR" => chrono::Weekday::Fri,
        "SA" => chrono::Weekday::Sat,
        "SU" => chrono::Weekday::Sun,
        _ => return now.timestamp(),
    };
    let today = now.date_naive();
    let current_weekday = today.weekday();
    let days_until = (target_num.num_days_from_monday() as i64
        - current_weekday.num_days_from_monday() as i64
        + 7)
        % 7;
    let target_date = if days_until == 0 && now.time() > time {
        today + chrono::Duration::days(7)
    } else if days_until == 0 {
        today
    } else {
        today + chrono::Duration::days(days_until)
    };
    target_date
        .and_time(time)
        .and_local_timezone(Local)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

fn next_occurrence_monthly(now: chrono::DateTime<Local>, target_day: u32, time: NaiveTime) -> i64 {
    let today = now.date_naive();
    let mut month = today.month();
    let mut year = today.year();
    if today.day() >= target_day && now.time() > time {
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    if let Some(d) = chrono::NaiveDate::from_ymd_opt(year, month, target_day) {
        d.and_time(time)
            .and_local_timezone(Local)
            .single()
            .map(|d| d.timestamp())
            .unwrap_or(0)
    } else {
        0
    }
}

// ════════════════════════════════════════════════════════════════════
// 解析辅助函数 / Parsing helper functions
// ════════════════════════════════════════════════════════════════════

/// 计算下个周X的日期 / Compute next week's weekday date
/// 下周X = 当前周的下一周的指定星期X
fn next_week_weekday(now: chrono::DateTime<Local>, target_day: &str, time: NaiveTime) -> i64 {
    let target_num = match target_day {
        "MO" => chrono::Weekday::Mon,
        "TU" => chrono::Weekday::Tue,
        "WE" => chrono::Weekday::Wed,
        "TH" => chrono::Weekday::Thu,
        "FR" => chrono::Weekday::Fri,
        "SA" => chrono::Weekday::Sat,
        "SU" => chrono::Weekday::Sun,
        _ => return now.timestamp(),
    };
    let today = now.date_naive();
    let current_weekday = today.weekday();
    // 下周X = 本周X + 7天 / Next week X = this week X + 7 days
    let days_this_week = (target_num.num_days_from_monday() as i64
        - current_weekday.num_days_from_monday() as i64
        + 7)
        % 7;
    let target_date = today + chrono::Duration::days(days_this_week + 7);
    target_date
        .and_time(time)
        .and_local_timezone(Local)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

/// 计算本周X的日期 / Compute this week's weekday date
fn this_week_weekday(now: chrono::DateTime<Local>, target_day: &str, time: NaiveTime) -> i64 {
    let target_num = match target_day {
        "MO" => chrono::Weekday::Mon,
        "TU" => chrono::Weekday::Tue,
        "WE" => chrono::Weekday::Wed,
        "TH" => chrono::Weekday::Thu,
        "FR" => chrono::Weekday::Fri,
        "SA" => chrono::Weekday::Sat,
        "SU" => chrono::Weekday::Sun,
        _ => return now.timestamp(),
    };
    let today = now.date_naive();
    let current_weekday = today.weekday();
    let days_until = (target_num.num_days_from_monday() as i64
        - current_weekday.num_days_from_monday() as i64
        + 7)
        % 7;
    let target_date = if days_until == 0 && now.time() > time {
        today + chrono::Duration::days(7) // 本周已过，下周同一天
    } else {
        today + chrono::Duration::days(days_until)
    };
    target_date
        .and_time(time)
        .and_local_timezone(Local)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

/// 解析"N天后" / Parse "N days later"
fn parse_days_later(msg: &str) -> Option<u32> {
    // "三天后" "3天后" "三十天后"
    // 排除"明天"/"后天"等：非数字前缀自然无法通过 start < i 检查
    // Exclude "明天"/"后天": non-digit prefix fails start < i check naturally
    let chars: Vec<char> = msg.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        // 检查"天后" / Check for "天后"
        if chars[i] == '天' && i + 1 < chars.len() && chars[i + 1] == '后' {
            // 向前找数字 / Look backwards for digits
            let mut start = i;
            while start > 0
                && (chars[start - 1].is_ascii_digit() || is_chinese_digit(chars[start - 1]))
            {
                start -= 1;
            }
            if start < i {
                let num_str: String = chars[start..i].iter().collect();
                if let Some(n) = parse_chinese_or_arabic(&num_str) {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// 解析中文或阿拉伯数字 / Parse Chinese or Arabic number
fn parse_chinese_or_arabic(s: &str) -> Option<u32> {
    // 先尝试阿拉伯数字 / Try Arabic first
    if let Ok(n) = s.trim().parse::<u32>() {
        return Some(n);
    }
    // 中文数字 / Chinese number
    let chars: Vec<char> = s.trim().chars().collect();
    if chars.is_empty() {
        return None;
    }
    // 简单支持 1-99 / Simple support for 1-99
    if chars.len() == 1 {
        return parse_digit(&chars[0].to_string());
    }
    // "三十" = 30, "十五" = 15, "二十" = 20
    if chars.len() == 2 {
        if chars[1] == '十' {
            // "X十" = X * 10
            return parse_digit(&chars[0].to_string()).map(|d| d * 10);
        }
        if chars[0] == '十' {
            // "十X" = 10 + X
            return parse_digit(&chars[1].to_string()).map(|d| 10 + d);
        }
    }
    // "三十一" = 31
    if chars.len() == 3 && chars[1] == '十' {
        let tens = parse_digit(&chars[0].to_string()).unwrap_or(0);
        let ones = parse_digit(&chars[2].to_string()).unwrap_or(0);
        return Some(tens * 10 + ones);
    }
    None
}

/// 计算下一个工作日 / Compute next workday
fn next_workday(now: chrono::DateTime<Local>, time: NaiveTime) -> i64 {
    let today = now.date_naive();
    for offset in 0..7 {
        let candidate = today + chrono::Duration::days(offset);
        let wd = candidate.weekday();
        let is_workday = matches!(
            wd,
            chrono::Weekday::Mon
                | chrono::Weekday::Tue
                | chrono::Weekday::Wed
                | chrono::Weekday::Thu
                | chrono::Weekday::Fri
        );
        if is_workday {
            let dt = candidate
                .and_time(time)
                .and_local_timezone(Local)
                .single()
                .map(|d| d.timestamp())
                .unwrap_or(0);
            if dt > now.timestamp() || offset > 0 {
                return dt;
            }
        }
    }
    now.timestamp()
}

/// 获取月份天数 / Get days in month
fn days_in_month(year: i32, month: u32) -> u32 {
    let next_month = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    let this_month = chrono::NaiveDate::from_ymd_opt(year, month, 1);
    match (this_month, next_month) {
        (Some(a), Some(b)) => (b - a).num_days() as u32,
        _ => 30,
    }
}

/// 计算下月X号的日期 / Compute next month's day X
fn next_month_day(now: chrono::DateTime<Local>, target_day: u32, time: NaiveTime) -> i64 {
    let today = now.date_naive();
    let mut month = today.month() + 1;
    let mut year = today.year();
    if month > 12 {
        month = 1;
        year += 1;
    }
    let day = target_day.min(days_in_month(year, month));
    if let Some(d) = chrono::NaiveDate::from_ymd_opt(year, month, day) {
        d.and_time(time)
            .and_local_timezone(Local)
            .single()
            .map(|d| d.timestamp())
            .unwrap_or(0)
    } else {
        0
    }
}

/// 从关键词后提取数字 / Extract number after keyword
fn extract_day_after_keyword(msg: &str, keyword: &str) -> Option<u32> {
    if let Some(pos) = msg.find(keyword) {
        let rest = &msg[pos + keyword.len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(d) = digits.parse::<u32>() {
            if (1..=31).contains(&d) {
                return Some(d);
            }
        }
    }
    None
}

/// 检测裸周X（有周X但无"每"前缀）/ Detect bare weekday (weekday without "每" prefix)
fn has_bare_weekday(msg: &str) -> bool {
    let weekday_patterns = [
        "周一",
        "周二",
        "周三",
        "周四",
        "周五",
        "周六",
        "周日",
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
        "星期天",
        "星期日",
    ];
    for pat in &weekday_patterns {
        if let Some(pos) = msg.find(pat) {
            // 检查前面是否有"每" / Check if preceded by "每"
            let before = &msg[..pos];
            if !before.ends_with('每') && !before.ends_with("每个") {
                return true;
            }
        }
    }
    false
}

// ════════════════════════════════════════════════════════════════════
// LLM 兜底 / LLM fallback
// ════════════════════════════════════════════════════════════════════

/// LLM 时间解析的系统提示 / System prompt for LLM time parsing
///
/// 当 [`parse_time`] 返回 `None` 时，把该提示发给任意 LLM（用户消息中
/// 附上当前 Unix 时间戳），再用 [`parse_llm_time_json`] 解析响应。
/// When [`parse_time`] returns `None`, send this prompt to any LLM
/// (include the current Unix timestamp in the user message), then parse
/// the response with [`parse_llm_time_json`].
pub const LLM_TIME_PARSE_PROMPT: &str = "\
你是一个时间解析助手。从用户的中文消息中提取时间信息，返回 JSON 格式。\
\n\n规则：\
\n1. 如果能确定重复模式，设置 rrule（RFC 5545 格式）和 one_shot=false\
\n2. 如果是一次性提醒，rrule 为空字符串，one_shot=true\
\n3. next_trigger_at 是 Unix 时间戳（秒）\
\n4. 如果无法从消息中提取任何时间信息，返回 {\"unable\": true}\
\n5. 当前时间戳在 user_prompt 中提供\
\n\n输出格式：\
\n{\"rrule\": \"FREQ=DAILY;BYHOUR=8;BYMINUTE=0\", \"next_trigger_at\": 1753000000, \"one_shot\": false}\
\n或\
\n{\"rrule\": \"\", \"next_trigger_at\": 1753000000, \"one_shot\": true}\
\n或\
\n{\"unable\": true}";

/// 解析 LLM 返回的时间 JSON / Parse an LLM time-JSON response
///
/// 响应格式见 [`LLM_TIME_PARSE_PROMPT`]。
/// See [`LLM_TIME_PARSE_PROMPT`] for the expected response format.
///
/// # 返回 / Returns
/// - `Some(TimeParseResult)` — 解析成功，confidence 固定为 0.8（低于规则精确匹配的 1.0）
/// - `None` — LLM 返回 `{"unable": true}` 或 JSON 无法解析
///
/// - `Some(TimeParseResult)` — parsed; confidence fixed at 0.8 (below the 1.0 of exact rule matches)
/// - `None` — LLM replied `{"unable": true}` or the JSON didn't parse
pub fn parse_llm_time_json(content: &str) -> Option<TimeParseResult> {
    // 简单 JSON 解析（不依赖 serde_json 以保持零依赖） / Simple JSON parsing (no serde_json dependency)
    // 查找 "unable": true
    if content.contains("\"unable\"") && content.contains("true") {
        return None;
    }

    // 提取 rrule / Extract rrule
    let rrule = extract_json_string(content, "rrule").unwrap_or_default();

    // 提取 next_trigger_at / Extract next_trigger_at
    let next_trigger_at = extract_json_number(content, "next_trigger_at")?;

    // 提取 one_shot / Extract one_shot
    let one_shot = extract_json_bool(content, "one_shot").unwrap_or(true);

    Some(TimeParseResult {
        rrule,
        next_trigger_at: next_trigger_at as i64,
        one_shot,
        confidence: 0.8, // LLM 解析置信度 — 信任 LLM 但略低于规则精确匹配
    })
}

/// 从 JSON 中提取字符串值 / Extract string value from JSON
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern)?;
    let after_key = &json[pos + pattern.len()..];
    let colon = after_key.find(':')?;
    let after_colon = &after_key[colon + 1..];
    let quote_start = after_colon.find('"')?;
    let after_quote = &after_colon[quote_start + 1..];
    let quote_end = after_quote.find('"')?;
    Some(after_quote[..quote_end].to_string())
}

/// 从 JSON 中提取数字值 / Extract number value from JSON
fn extract_json_number(json: &str, key: &str) -> Option<f64> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern)?;
    let after_key = &json[pos + pattern.len()..];
    let colon = after_key.find(':')?;
    let after_colon = &after_key[colon + 1..].trim_start();
    let end = after_colon
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .unwrap_or(after_colon.len());
    after_colon[..end].parse().ok()
}

/// 从 JSON 中提取布尔值 / Extract boolean value from JSON
fn extract_json_bool(json: &str, key: &str) -> Option<bool> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern)?;
    let after_key = &json[pos + pattern.len()..];
    let colon = after_key.find(':')?;
    let after_colon = &after_key[colon + 1..].trim_start();
    if after_colon.starts_with("true") {
        Some(true)
    } else if after_colon.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daily() {
        let r = parse_time("每天早上8点提醒我开会").unwrap();
        assert!(r.rrule.contains("FREQ=DAILY"));
        assert!(r.rrule.contains("BYHOUR=8"));
        assert!(!r.one_shot);
        assert!(r.confidence >= 0.5);
    }

    #[test]
    fn test_weekly() {
        let r = parse_time("每周三下午3点开会").unwrap();
        assert!(r.rrule.contains("FREQ=WEEKLY"));
        assert!(r.rrule.contains("BYDAY=WE"));
        assert!(r.rrule.contains("BYHOUR=15"));
    }

    #[test]
    fn test_monthly() {
        let r = parse_time("每月1号还款").unwrap();
        assert!(r.rrule.contains("FREQ=MONTHLY"));
        assert!(r.rrule.contains("BYMONTHDAY=1"));
    }

    #[test]
    fn test_one_shot() {
        let r = parse_time("明天下午2点提醒我").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
    }

    #[test]
    fn test_evening() {
        let r = parse_time("每天晚上9点").unwrap();
        assert!(r.rrule.contains("BYHOUR=21"));
    }

    #[test]
    fn test_no_time_returns_none() {
        // 无时间线索应返回 None（可触发 LLM 兜底）
        // No time clues → None (LLM fallback is up to the caller)
        assert!(parse_time("提醒我开会").is_none());
    }

    #[test]
    fn test_precise_minute() {
        let r = parse_time("每天下午3点20提醒我").unwrap();
        assert!(r.rrule.contains("BYHOUR=15"));
        assert!(r.rrule.contains("BYMINUTE=20"));
    }

    #[test]
    fn test_half_hour() {
        let r = parse_time("每天3点半提醒我").unwrap();
        assert!(r.rrule.contains("BYMINUTE=30"));
    }

    #[test]
    fn test_precise_no_ampm() {
        let r = parse_time("每天早上8点05分").unwrap();
        assert!(r.rrule.contains("BYHOUR=8"));
    }

    // ── 扩展模式测试 / Extended pattern tests ──

    #[test]
    fn test_every_other_day() {
        // 隔天 / Every other day
        let r = parse_time("隔天早上8点提醒我").unwrap();
        assert!(r.rrule.contains("FREQ=DAILY"));
        assert!(r.rrule.contains("INTERVAL=2"));
        assert!(r.rrule.contains("BYHOUR=8"));
        assert!(!r.one_shot);
    }

    #[test]
    fn test_workday() {
        // 工作日 / Workdays
        let r = parse_time("工作日每天早上9点提醒我").unwrap();
        assert!(r.rrule.contains("FREQ=WEEKLY"));
        assert!(r.rrule.contains("BYDAY=MO,TU,WE,TH,FR"));
        assert!(r.rrule.contains("BYHOUR=9"));
        assert!(!r.one_shot);
    }

    #[test]
    fn test_next_week_weekday() {
        // 下周一 / Next Monday
        let r = parse_time("下周一下午3点提醒我开会").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
        assert!(r.next_trigger_at > 0);
    }

    #[test]
    fn test_this_week_weekday() {
        // 这周三 / This Wednesday
        let r = parse_time("这周三下午2点开会").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
        assert!(r.next_trigger_at > 0);
    }

    #[test]
    fn test_days_later_arabic() {
        // 3天后 / 3 days later
        let r = parse_time("3天后提醒我交报告").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
        assert!(r.next_trigger_at > 0);
    }

    #[test]
    fn test_days_later_chinese() {
        // 三天后 / Three days later
        let r = parse_time("三天后提醒我交报告").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
        assert!(r.next_trigger_at > 0);
    }

    #[test]
    fn test_month_end() {
        // 月底 / End of month
        let r = parse_time("月底提醒我还信用卡").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
        assert!(r.next_trigger_at > 0);
    }

    #[test]
    fn test_next_month_day() {
        // 下月15号 / Next month 15th
        let r = parse_time("下月15号提醒我还款").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
        assert!(r.next_trigger_at > 0);
    }

    #[test]
    fn test_bare_weekday() {
        // 周三（无"每"前缀）/ Wednesday (no "每" prefix)
        let r = parse_time("周三下午3点提醒我开会").unwrap();
        assert!(r.one_shot);
        assert!(r.rrule.is_empty());
        assert!(r.next_trigger_at > 0);
    }

    #[test]
    fn test_confidence_exact_time() {
        // 精确时间 → confidence = 1.0
        let r = parse_time("每天早上8点提醒我").unwrap();
        assert_eq!(r.confidence, 1.0);
    }

    #[test]
    fn test_confidence_default_time() {
        // 默认时间 → confidence = 0.5
        let r = parse_time("每天提醒我").unwrap();
        assert_eq!(r.confidence, 0.5);
    }

    #[test]
    fn test_no_reminder_no_time() {
        // 无时间线索 → None
        assert!(parse_time("帮我买个咖啡").is_none());
    }

    // ── LLM JSON 解析测试 / LLM JSON parsing tests ──

    #[test]
    fn test_llm_json_one_shot() {
        let r = parse_llm_time_json(
            r#"{"rrule": "", "next_trigger_at": 1753000000, "one_shot": true}"#,
        )
        .unwrap();
        assert!(r.one_shot);
        assert_eq!(r.next_trigger_at, 1753000000);
        assert_eq!(r.confidence, 0.8);
    }

    #[test]
    fn test_llm_json_recurring() {
        let r = parse_llm_time_json(
            r#"{"rrule": "FREQ=DAILY;BYHOUR=14;BYMINUTE=30", "next_trigger_at": 1753000000, "one_shot": false}"#,
        )
        .unwrap();
        assert!(!r.one_shot);
        assert!(r.rrule.contains("FREQ=DAILY"));
    }

    #[test]
    fn test_llm_json_unable() {
        assert!(parse_llm_time_json(r#"{"unable": true}"#).is_none());
    }

    #[test]
    fn test_extract_json_string() {
        let json = r#"{"rrule": "FREQ=DAILY", "x": 1}"#;
        assert_eq!(
            extract_json_string(json, "rrule").as_deref(),
            Some("FREQ=DAILY")
        );
    }

    #[test]
    fn test_extract_json_number() {
        let json = r#"{"next_trigger_at": 1753000000, "x": 1}"#;
        assert_eq!(
            extract_json_number(json, "next_trigger_at"),
            Some(1753000000.0)
        );
    }

    #[test]
    fn test_extract_json_bool() {
        let json = r#"{"one_shot": true, "x": 1}"#;
        assert_eq!(extract_json_bool(json, "one_shot"), Some(true));
        let json2 = r#"{"one_shot": false}"#;
        assert_eq!(extract_json_bool(json2, "one_shot"), Some(false));
    }

    #[test]
    fn test_parse_days_later() {
        assert_eq!(parse_days_later("3天后"), Some(3));
        assert_eq!(parse_days_later("三天后"), Some(3));
        assert_eq!(parse_days_later("十天后"), Some(10));
        assert_eq!(parse_days_later("三十天后"), Some(30));
        assert_eq!(parse_days_later("明天"), None);
    }

    #[test]
    fn test_days_in_month() {
        assert_eq!(days_in_month(2026, 1), 31);
        assert_eq!(days_in_month(2026, 2), 28); // 2026 非闰年
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn test_has_bare_weekday() {
        assert!(has_bare_weekday("周三下午3点"));
        assert!(has_bare_weekday("周五开会"));
        assert!(!has_bare_weekday("每周三"));
        assert!(!has_bare_weekday("每天"));
    }
}
