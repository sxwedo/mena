
use super::{ConfigColor, DEFAULT_CONFIG_TEMPLATE, Settings};

#[test]
fn default_template_is_valid_and_empty() {
    let settings: Settings =
        toml::from_str(DEFAULT_CONFIG_TEMPLATE).expect("default config should parse");
    assert!(settings.agent.custom.is_empty());
    let colors = settings.ui.session_detail.colors;
    assert_eq!(colors.user_header, ConfigColor::Rgb(0xd3, 0xaa, 0x6e));
    assert_eq!(colors.user_content, ConfigColor::Rgb(0xe1, 0xe6, 0xeb));
    assert_eq!(colors.assistant_header, ConfigColor::Rgb(0x7c, 0xa7, 0xd9));
    assert_eq!(
        colors.tool_result_content,
        ConfigColor::Rgb(0xa8, 0xb0, 0xba)
    );
}

#[test]
fn custom_agent_config_round_trips() {
    let source = r#"
[agent.custom.my_agent]
executables = ["my-agent", "my-agent.exe"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
"#;
    let settings: Settings = toml::from_str(source).expect("custom settings should parse");
    let serialized = toml::to_string_pretty(&settings).expect("settings should serialize");
    let reparsed: Settings = toml::from_str(&serialized).expect("serialized settings parse");
    let custom = &reparsed.agent.custom["my_agent"];
    assert_eq!(custom.executables, ["my-agent", "my-agent.exe"]);
    assert_eq!(custom.command_contains, ["--agent-mode"]);
    assert_eq!(custom.resume, ["my-agent", "resume", "{session}"]);
}

#[test]
fn session_detail_colors_accept_names_indexed_colors_and_rgb() {
    let source = r##"
[ui.session_detail.colors]
user_header = "white"
user_content = "#a1B2c3"
assistant_header = "ansi:45"
assistant_content = "indexed:200"
"##;

    let settings: Settings = toml::from_str(source).expect("color settings should parse");
    let colors = settings.ui.session_detail.colors;

    assert_eq!(colors.user_header, ConfigColor::White);
    assert_eq!(colors.user_content, ConfigColor::Rgb(0xa1, 0xb2, 0xc3));
    assert_eq!(colors.assistant_header, ConfigColor::Indexed(45));
    assert_eq!(colors.assistant_content, ConfigColor::Indexed(200));
    assert_eq!(colors.tool_call_header, ConfigColor::Rgb(0x79, 0xb8, 0xc7));
}

#[test]
fn invalid_session_detail_color_fails_with_an_actionable_error() {
    let error =
        toml::from_str::<Settings>("[ui.session_detail.colors]\nuser_header = \"ultraviolet\"\n")
            .expect_err("unknown colors must fail");

    assert!(
        error
            .to_string()
            .contains("unsupported color `ultraviolet`")
    );
    assert!(error.to_string().contains("#RRGGBB"));
}
