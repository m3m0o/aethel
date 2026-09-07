mod model;
mod raw;
mod substitute;
mod toml_template;

pub use model::PreparedRequest;
pub use raw::parse as parse_raw;
pub use substitute::placeholders;
pub use toml_template::parse as parse_toml;

#[cfg(test)]
mod tests {
    use super::{parse_raw, parse_toml};
    use std::collections::BTreeMap;

    #[test]
    fn raw_and_toml_templates_produce_equivalent_requests() {
        let values = BTreeMap::from([
            (String::from("username"), String::from("admin")),
            (String::from("password"), String::from("secret")),
        ]);
        let raw = "POST /login HTTP/1.1\nHost: example.test\nContent-Type: application/json\nX-User: {{username}}\n\n{\"password\":\"{{password}}\"}";
        let toml = "method = 'POST'\npath = '/login'\n\n[headers]\nHost = 'example.test'\nContent-Type = 'application/json'\nX-User = '{{username}}'\n\nbody = '{\"password\":\"{{password}}\"}'";
        assert_eq!(
            parse_raw(raw, Some("https://example.test"), &values).unwrap(),
            parse_toml(toml, Some("https://example.test"), &values).unwrap()
        );
    }
}
