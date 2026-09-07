use std::collections::BTreeMap;

use anyhow::{Result, bail};

pub fn substitute(input: &str, values: &BTreeMap<String, String>) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let end = after_start
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unterminated placeholder"))?;
        let name = &after_start[..end];
        if name.is_empty()
            || !name
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || value == '_')
        {
            bail!("invalid placeholder name: {name}");
        }
        let value = values
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unresolved placeholder: {name}"))?;
        output.push_str(value);
        rest = &after_start[end + 2..];
    }
    output.push_str(rest);
    if output.contains("{{") || output.contains("}}") {
        bail!("unresolved placeholder syntax");
    }
    Ok(output)
}

pub fn placeholders(input: &str) -> Result<std::collections::BTreeSet<String>> {
    let mut names = std::collections::BTreeSet::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        let end = after_start
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unterminated placeholder"))?;
        let name = &after_start[..end];
        if name.is_empty()
            || !name
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || value == '_')
        {
            bail!("invalid placeholder name: {name}");
        }
        names.insert(name.to_owned());
        rest = &after_start[end + 2..];
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::substitute;
    use std::collections::BTreeMap;

    #[test]
    fn substitutes_named_values() {
        let values = BTreeMap::from([(String::from("user"), String::from("admin"))]);
        assert_eq!(
            substitute("hello {{user}}", &values).unwrap(),
            "hello admin"
        );
    }

    #[test]
    fn rejects_missing_values() {
        assert!(substitute("hello {{user}}", &BTreeMap::new()).is_err());
    }
}
