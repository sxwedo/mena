
use super::{RawValue, classify_value};

#[test]
fn classifies_environment_references_without_treating_them_as_literals() {
    assert!(matches!(
        classify_value("${API_TOKEN}"),
        RawValue::Environment(environment) if environment == "API_TOKEN"
    ));
    assert!(matches!(
        classify_value("Bearer ${API_TOKEN}"),
        RawValue::EnvironmentTemplate { template, environments }
            if template == "Bearer ${API_TOKEN}" && environments == ["API_TOKEN"]
    ));
    assert!(matches!(
        classify_value("{env:API_TOKEN}"),
        RawValue::Environment(environment) if environment == "API_TOKEN"
    ));
    assert!(matches!(
        classify_value("${API_HOST:-https://example.test}"),
        RawValue::EnvironmentTemplate { environments, .. }
            if environments == ["API_HOST"]
    ));
    assert!(matches!(
        classify_value("!security lookup token"),
        RawValue::DynamicCommand
    ));
}
