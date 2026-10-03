//! Display formatting shared by the TUI and headless output.
use ergotop_core::model::nano_to_erg;

pub fn erg(nano: u64) -> String {
    format!("{:.2}", nano_to_erg(nano))
}

pub fn fee(nano: u64) -> String {
    format!("{:.4}", nano_to_erg(nano))
}

pub fn bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.2} MB", n as f64 / (1024.0 * 1024.0))
    }
}

pub fn age(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    }
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

pub fn short_addr(a: &str) -> String {
    let n = a.chars().count();
    if n <= 16 {
        return a.to_string();
    }
    let head: String = a.chars().take(8).collect();
    let tail: String = a.chars().skip(n - 6).collect();
    format!("{head}…{tail}")
}

pub fn trunc(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_amounts() {
        assert_eq!(erg(11_887_500_000), "11.89");
        assert_eq!(fee(1_500_000), "0.0015");
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(412), "412 B");
        assert_eq!(bytes(2150), "2.1 KB");
        assert_eq!(bytes(2_097_152), "2.00 MB");
    }

    #[test]
    fn formats_age() {
        assert_eq!(age(5_000), "5s");
        assert_eq!(age(252_000), "4m 12s");
        assert_eq!(age(7_380_000), "2h 03m");
    }

    #[test]
    fn formats_ids_and_numbers() {
        assert_eq!(thousands(1_886_101), "1,886,101");
        assert_eq!(thousands(999), "999");
        assert_eq!(short_id("abcdef0123456789"), "abcdef01");
        assert_eq!(short_addr("9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq"), "9guaDYhH…Ym3Rsq");
        assert_eq!(short_addr("4MQyMKvMbnCJG3aJ"), "4MQyMKvMbnCJG3aJ");
        assert_eq!(trunc("Rosen Bridge", 5), "Rosen");
    }
}
