//! Decode subprocess / OS error bytes on Windows (often GBK on zh-CN systems).

/// Decode console or subprocess output (UTF-8 first, then GBK on Windows).
pub fn decode_console_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    #[cfg(windows)]
    {
        use encoding_rs::GBK;
        let (cow, _, _) = GBK.decode(bytes);
        cow.into_owned()
    }
    #[cfg(not(windows))]
    {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Strip mojibake suffixes from libgit2 / Win32 messages shown in the UI.
pub fn sanitize_transport_message(msg: &str) -> String {
    let msg = msg.trim();
    if let Some(idx) = msg.find("failed to resolve address for ") {
        let tail = &msg[idx..];
        let host = tail
            .strip_prefix("failed to resolve address for ")
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .trim();
        if !host.is_empty() {
            return format!("failed to resolve address for {host}");
        }
    }
    if msg.is_ascii() {
        return msg.to_string();
    }
    let bytes: Vec<u8> = msg.bytes().collect();
    decode_console_bytes(&bytes)
}
