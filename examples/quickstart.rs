// SPDX-License-Identifier: MIT
//! Quickstart — parse a few expressions and print the results.

use chrono::{Local, TimeZone};

fn main() {
    let inputs = [
        "每天早上8点提醒我喝水",
        "每周三下午3点站会",
        "每月1号还款",
        "工作日每天早上9点",
        "隔天早上8点",
        "明天下午2点提醒我拿快递",
        "下周一下午3点开会",
        "三天后提醒我交报告",
        "月底提醒我还信用卡",
    ];

    for input in inputs {
        match chinese_rrule::parse_time(input) {
            Some(r) => {
                let kind = if r.one_shot { "one-shot" } else { "recurring" };
                let when = Local
                    .timestamp_opt(r.next_trigger_at, 0)
                    .single()
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "?".into());
                println!("{input}");
                println!(
                    "  → [{}] rrule={:?} next={} (confidence {:.1})",
                    kind,
                    if r.rrule.is_empty() { "—" } else { &r.rrule },
                    when,
                    r.confidence
                );
            }
            None => {
                println!("{input}\n  → no time clues (try the LLM fallback)");
            }
        }
    }
}
