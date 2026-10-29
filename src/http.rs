//! Blocking HTTP GET over the WinRT `HttpClient`.
//! Call only from a thread started with [`crate::worker::spawn`].

use windows::core::{Result, HSTRING};
use windows::Foundation::Uri;
use windows::Storage::Streams::DataReader;
use windows::Web::Http::HttpClient;

pub fn get_string(url: &str) -> Result<String> {
    let client = HttpClient::new()?;
    client
        .DefaultRequestHeaders()?
        .UserAgent()?
        .TryParseAdd(&HSTRING::from("Ada/0.1 (Windows)"))?;
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    let body = client.GetStringAsync(&uri)?.get()?;
    Ok(body.to_string_lossy())
}

/// Downloads the raw body (e.g. an image). Non-2xx statuses are errors.
pub fn get_bytes(url: &str) -> std::result::Result<Vec<u8>, String> {
    fetch_bytes(url).map_err(|e| e.message())
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let client = HttpClient::new()?;
    client
        .DefaultRequestHeaders()?
        .UserAgent()?
        .TryParseAdd(&HSTRING::from("Ada/0.1 (Windows)"))?;
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    let buffer = client.GetBufferAsync(&uri)?.get()?;
    let reader = DataReader::FromBuffer(&buffer)?;
    let mut bytes = vec![0u8; reader.UnconsumedBufferLength()? as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

pub fn get_json(url: &str) -> std::result::Result<serde_json::Value, String> {
    let body = get_string(url).map_err(|e| e.message())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// Percent-encodes a query parameter value.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
