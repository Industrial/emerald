//! Real end-to-end proof of plan 77's `leaf-doc-comment-extraction`
//! acceptance criteria — spawns the actual compiled `emerald` binary,
//! never calling `emerald_parser`/`emerald_cli`'s internal `doc_runner`
//! module directly.

use std::path::PathBuf;
use std::process::Command;

fn fresh_dir(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-cli-doc-subcommand-{tag}-{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  dir
}

fn repo_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn emerald_doc_on_a_real_file_extracts_names_signatures_and_doc_text() {
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("doc")
    .arg("examples/doc_comments.em")
    .current_dir(repo_root())
    .output()
    .unwrap();

  assert!(output.status.success(), "{output:?}");
  let stdout = String::from_utf8_lossy(&output.stdout);

  // Documented declarations: name + real signature + doc text all
  // present.
  assert!(
    stdout.contains("fn add(a: Int64, b: Int64): Int64"),
    "{stdout}"
  );
  assert!(stdout.contains("Adds two integers together."), "{stdout}");
  assert!(stdout.contains("class Point"), "{stdout}");
  assert!(
    stdout.contains("A point in 2D space, with a Manhattan-distance helper."),
    "{stdout}"
  );
  assert!(stdout.contains("fn manhattan(): Int64"), "{stdout}");
  assert!(
    stdout.contains("The sum of the absolute values of both coordinates."),
    "{stdout}"
  );
  assert!(stdout.contains("module NumberUtils"), "{stdout}");
  assert!(stdout.contains("Namespaced integer helpers."), "{stdout}");
  assert!(stdout.contains("fn square(n: Int64): Int64"), "{stdout}");
  assert!(stdout.contains("Squares an integer."), "{stdout}");
  assert!(
    stdout.contains("enum Payment = Cash(Int64) | Card(Int64)"),
    "{stdout}"
  );
  assert!(stdout.contains("How a purchase was paid for."), "{stdout}");

  // Undocumented/deliberately-orphaned declarations are absent
  // entirely.
  assert!(!stdout.contains("fn subtract"), "{stdout}");
  assert!(!stdout.contains("fn multiply"), "{stdout}");
  assert!(!stdout.contains("doubled_manhattan"), "{stdout}");
}

#[test]
fn emerald_doc_writes_to_a_file_with_the_o_flag() {
  let dir = fresh_dir("o-flag");
  let out_path = dir.join("api.md");

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("doc")
    .arg("examples/doc_comments.em")
    .arg("-o")
    .arg(&out_path)
    .current_dir(repo_root())
    .output()
    .unwrap();

  assert!(output.status.success(), "{output:?}");
  assert!(String::from_utf8_lossy(&output.stdout).is_empty());
  let written = std::fs::read_to_string(&out_path).unwrap();
  assert!(written.contains("Adds two integers together."), "{written}");

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn emerald_doc_on_an_undocumented_file_reports_nothing_found() {
  let dir = fresh_dir("undocumented");
  let path = dir.join("plain.em");
  std::fs::write(
    &path,
    "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("doc")
    .arg(&path)
    .output()
    .unwrap();

  assert!(output.status.success(), "{output:?}");
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(stdout.contains("No `##` doc comments found."), "{stdout}");
  assert!(!stdout.contains("fn add"), "{stdout}");

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn emerald_doc_on_a_directory_walks_every_em_file() {
  let dir = fresh_dir("directory");
  std::fs::write(
    dir.join("a.em"),
    "## Doc for a.\nfn a_fn(): Int64 do\n  1\nend\n",
  )
  .unwrap();
  std::fs::write(
    dir.join("b.em"),
    "## Doc for b.\nfn b_fn(): Int64 do\n  2\nend\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("doc")
    .arg(&dir)
    .output()
    .unwrap();

  assert!(output.status.success(), "{output:?}");
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(stdout.contains("Doc for a."), "{stdout}");
  assert!(stdout.contains("Doc for b."), "{stdout}");

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn emerald_doc_on_a_syntax_error_reports_it_and_exits_nonzero() {
  let dir = fresh_dir("syntax-error");
  let path = dir.join("broken.em");
  std::fs::write(&path, "fn broken(: Int64 do\nend\n").unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("doc")
    .arg(&path)
    .output()
    .unwrap();

  assert!(!output.status.success());
  assert!(!String::from_utf8_lossy(&output.stderr).is_empty());

  std::fs::remove_dir_all(&dir).ok();
}
