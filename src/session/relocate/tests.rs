use super::*;
use std::io::Write as _;
use tempfile::tempdir;

fn age(path: &Path) {
    let file = File::options().write(true).open(path).expect("open");
    file.set_modified(SystemTime::now() - Duration::from_mins(2))
        .expect("age file");
}

#[test]
fn preview_and_commit_move_the_store_and_rewrite_only_cwd() {
    let temp = tempdir().expect("temp");
    let home = temp.path();
    let from = PathBuf::from("/work/old-project");
    let to = temp.path().join("new-project");
    fs::create_dir_all(&to).expect("dest project");
    let key = claude_project_key(&from).expect("key");
    let store = home.join(".claude/projects").join(key);
    fs::create_dir_all(&store).expect("store");
    let transcript = store.join("abc.jsonl");
    let mut file = File::create(&transcript).expect("transcript");
    writeln!(
            file,
            r#"{{"sessionId":"abc","cwd":"/work/old-project","message":{{"content":"cwd /work/old-project stays"}}}}"#
        )
        .expect("write");
    drop(file);
    age(&transcript);

    let preview = preview_claude_relocation(home, &from, &to).expect("preview");
    assert_eq!(preview.session_files, 1);
    let backup = commit_claude_relocation(home, &from, &to, &preview.token).expect("commit");
    assert!(backup.join("original").join("abc.jsonl").is_file());
    let moved = home
        .join(".claude/projects")
        .join(claude_project_key(&to).expect("new key"))
        .join("abc.jsonl");
    let text = fs::read_to_string(moved).expect("read moved");
    assert!(text.contains(&format!(r#""cwd":"{}""#, to.display())));
    assert!(text.contains("cwd /work/old-project stays"));
    assert!(!store.exists());
}

#[test]
fn colliding_encodings_are_refused() {
    let temp = tempdir().expect("temp");
    let error = preview_claude_relocation(temp.path(), Path::new("/a/b-c"), Path::new("/a-b/c"))
        .expect_err("collision");
    assert!(format!("{error:#}").contains("same Claude project directory"));
}
