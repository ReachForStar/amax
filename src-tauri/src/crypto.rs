//! Windows DPAPI 加密模块
//!
//! 使用当前用户凭据加密凭据，只有同一用户在同一机器上才能解密。
//! 加密后的数据以 hex 编码存入 SQLite。

#[cfg(windows)]
use std::ptr;
#[cfg(windows)]
use windows_sys::Win32::Foundation::LocalFree;
#[cfg(windows)]
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData,
};

/// DPAPI 加密 → hex 字符串
#[cfg(windows)]
pub fn encrypt(plaintext: &[u8]) -> Result<String, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plaintext.len()).map_err(|_| "凭据长度超出 DPAPI 限制")?,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    let ok = unsafe {
        CryptProtectData(
            &input,
            windows_sys::w!("AMAX Dashboard Secret"),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        return Err("DPAPI 加密失败".into());
    }

    let encrypted = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let encoded = hex_encode(encrypted);
    unsafe { LocalFree(output.pbData as *mut std::ffi::c_void) };
    Ok(encoded)
}

/// DPAPI 解密 (输入 hex 字符串) → 原始字节
#[cfg(windows)]
pub fn decrypt(hex: &str) -> Result<Vec<u8>, String> {
    let encrypted = hex_decode(hex)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(encrypted.len()).map_err(|_| "加密数据长度超出 DPAPI 限制")?,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        return Err("DPAPI 解密失败 (可能是不同用户或机器)".into());
    }

    let decrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData as *mut std::ffi::c_void) };
    Ok(decrypted)
}

/// 非 Windows 平台不提供不安全的明文降级。
#[cfg(not(windows))]
pub fn encrypt(_plaintext: &[u8]) -> Result<String, String> {
    Err("当前平台不支持 DPAPI 安全存储".into())
}

#[cfg(not(windows))]
pub fn decrypt(_hex: &str) -> Result<Vec<u8>, String> {
    Err("当前平台不支持 DPAPI 安全存储".into())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("写入 String 不会失败");
    }
    encoded
}

fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
    // 奇数长度即非法；长度校验与切分成对由 as_chunks 的余片一次性表达，不再分开判
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err("无效的 hex 字符串".into());
    }

    pairs
        .iter()
        .map(|pair| {
            let high = decode_nibble(pair[0])?;
            let low = decode_nibble(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn decode_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("hex 解码失败: 包含非法字符".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{hex_decode, hex_encode};

    #[test]
    fn hex_round_trip() {
        let value = b"session=abc123";
        assert_eq!(hex_decode(&hex_encode(value)).unwrap(), value);
    }

    #[test]
    fn invalid_hex_returns_error_without_panicking() {
        assert!(hex_decode("abc").is_err());
        assert!(hex_decode("你好").is_err());
        assert!(hex_decode("zz").is_err());
    }
}
