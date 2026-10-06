use crate::vm::*;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub static FNS: &[Native] = &[
    Native { name: "now", f: now },
    Native { name: "ms", f: ms },
    Native { name: "clock", f: clock },
    Native { name: "sleep", f: sleep },
    Native { name: "iso", f: iso },
];

fn start() -> Instant {
    static T0: OnceLock<Instant> = OnceLock::new();
    *T0.get_or_init(Instant::now)
}

fn unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn now(_: &mut Vm, a: Args) -> R {
    a.bind([])?;
    Ok(Value::Float(unix()))
}

fn ms(_: &mut Vm, a: Args) -> R {
    a.bind([])?;
    Ok(Value::Int((unix() * 1000.0) as i64))
}

// seconds since program start, monotonic
fn clock(_: &mut Vm, a: Args) -> R {
    a.bind([])?;
    Ok(Value::Float(start().elapsed().as_secs_f64()))
}

fn sleep(vm: &mut Vm, a: Args) -> R {
    let [s] = a.bind(["seconds"])?;
    let s = need(s, "seconds")?.num("seconds")?;
    if !(0.0..=1e7).contains(&s) {
        return Err(value_err("sleep needs 0..10000000 seconds"));
    }
    let _ = std::io::Write::flush(&mut vm.out);
    std::thread::sleep(Duration::from_secs_f64(s));
    Ok(Value::Nil)
}

// unix seconds to "YYYY-MM-DDTHH:MM:SSZ" (UTC)
pub fn iso_utc(t: f64) -> String {
    let secs = t.floor() as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil-from-days, Howard Hinnant
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn iso(_: &mut Vm, a: Args) -> R {
    let [t] = a.bind(["t"])?;
    let t = match opt(t) {
        Some(v) => v.num("t")?,
        None => unix(),
    };
    Ok(Value::str(iso_utc(t)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_dates() {
        assert_eq!(super::iso_utc(0.0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso_utc(951_782_400.0), "2000-02-29T00:00:00Z");
        assert_eq!(super::iso_utc(1_791_244_800.0), "2026-10-06T00:00:00Z");
    }
}
