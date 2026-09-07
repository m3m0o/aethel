use reqwest::header::HeaderMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    pub method: reqwest::Method,
    pub url: reqwest::Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}
