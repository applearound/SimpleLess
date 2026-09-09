const SERVICE: &str = "com.yezhou.simpleless";
const ACCOUNT: &str = "dashscope-api-key";

pub fn set_api_key(key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    entry.set_password(key).map_err(|e| e.to_string())
}

pub fn get_api_key() -> Result<String, String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    entry.get_password().map_err(|e| e.to_string())
}
