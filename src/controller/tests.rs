use super::split_session_selector;
use std::collections::BTreeMap;

#[test]
fn grok_resume_targets_split_as_a_known_provider() {
    let custom = BTreeMap::new();
    let (provider, id) =
        split_session_selector("grok:01a05b12-60ae-7790-8716-9293782180a9", &custom);
    assert_eq!(provider, Some("grok"));
    assert_eq!(id, "01a05b12-60ae-7790-8716-9293782180a9");
}
