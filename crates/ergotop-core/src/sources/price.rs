//! ERG/USD price from the SigmaUSD oracle frontend.
use super::{Result, SourceError};

pub const PRICE_URL: &str = "https://erg-oracle-ergusd.spirepools.com/frontendData";

/// The oracle returns a JSON object encoded as a JSON string; accept both forms.
pub fn parse_price(text: &str) -> Option<f64> {
    let text = text.trim();
    let inner: String = if text.starts_with('"') {
        serde_json::from_str::<String>(text).ok()?
    } else {
        text.to_string()
    };
    let v: serde_json::Value = serde_json::from_str(&inner).ok()?;
    v.get("latest_price")?.as_f64()
}

pub async fn fetch_price(http: &reqwest::Client, url: &str) -> Result<f64> {
    let text = http.get(url).send().await?.text().await?;
    parse_price(&text).ok_or_else(|| SourceError::Parse("latest_price missing".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_json_string() {
        let text = r#""{\"title\":\"Erg-USD\",\"latest_price\":0.3227559441177931}""#;
        assert_eq!(parse_price(text), Some(0.3227559441177931));
    }

    #[test]
    fn parses_plain_json_and_rejects_garbage() {
        assert_eq!(parse_price(r#"{"latest_price":1.5}"#), Some(1.5));
        assert_eq!(parse_price("<html>"), None);
    }
}
