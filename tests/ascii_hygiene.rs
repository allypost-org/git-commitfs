mod common;

use common::{Repo, assert_failure, assert_success};

fn run_check(repo: &Repo) -> std::process::Output {
    repo.command("bash")
        .arg(repo.root.join("scripts/check-ascii"))
        .output()
        .expect("ASCII hygiene check")
}

#[test]
fn hygiene_allows_ascii_and_language_unicode_but_rejects_decorative_glyphs() {
    let repo = Repo::new();
    repo.write(
        "scripts/check-ascii",
        include_bytes!("../scripts/check-ascii"),
    );
    repo.commit_all();
    repo.write("text", "ASCII - -> ... 'quotes'\ncaf\u{e9}\n".as_bytes());
    repo.git(&["add", "text"]);
    assert_success(&run_check(&repo));
    for glyph in [
        "\u{2013}", "\u{2014}", "\u{2018}", "\u{2019}", "\u{201c}", "\u{201d}", "\u{2022}",
        "\u{2026}", "\u{2192}", "\u{a0}",
    ] {
        repo.write("text", format!("forbidden {glyph}\n").as_bytes());
        repo.git(&["add", "text"]);
        assert_failure(&run_check(&repo), 1, "Typographic glyphs");
    }
}
