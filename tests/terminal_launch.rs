#[cfg(all(feature = "desktop", unix))]
mod unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn ticket(
        executable: &str,
        arguments: &[&str],
        mode: u32,
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::Builder::new()
            .prefix("mena-resume-")
            .tempdir()
            .expect("private ticket directory");
        let runner = directory.path().join("Resume.tool");
        fs::copy(env!("CARGO_BIN_EXE_mena-terminal-launch"), &runner).expect("launcher");
        let request = directory.path().join("launch.json");
        fs::write(&request, serde_json::to_vec(&serde_json::json!({"executable": executable, "arguments": arguments, "directory": std::env::current_dir().expect("project")})).expect("encode")).expect("request");
        fs::set_permissions(request, fs::Permissions::from_mode(mode)).expect("private request");
        (directory, runner)
    }

    #[test]
    fn external_resume_executes_literal_argv_and_consumes_private_ticket() {
        let (directory, runner) = ticket(
            "/usr/bin/printf",
            &["%s", "spaces 'quotes' ;$(touch NEVER_EXECUTE) `echo nope`"],
            0o600,
        );
        let output = Command::new(&runner).output().expect("launch");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "spaces 'quotes' ;$(touch NEVER_EXECUTE) `echo nope`"
        );
        assert!(!directory.path().exists());
        assert!(Command::new(runner).output().is_err());
        let (directory, runner) = ticket("/bin/pwd", &[], 0o600);
        let expected = std::env::current_dir()
            .expect("project")
            .canonicalize()
            .expect("cwd");
        let output = Command::new(runner).output().expect("pwd");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected.to_string_lossy()
        );
        assert!(!directory.path().exists());
    }

    #[test]
    fn external_resume_rejects_public_requests_without_executing() {
        let (directory, runner) = ticket("/usr/bin/printf", &["SHOULD_NOT_RUN"], 0o644);
        let output = Command::new(runner).output().expect("run validation");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(directory.path().join("launch.json").exists());
    }
}
