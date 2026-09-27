//! The README's tapes are tapes: every one of its `elixir` blocks parses.

#[test]
fn every_tape_in_the_readme_parses() {
    let readme = include_str!("../README.md");
    let tapes: Vec<&str> =
        readme.split("```elixir\n").skip(1).map(|block| block.split("```").next().unwrap()).collect();

    assert!(tapes.len() >= 2, "the README has tapes");
    for tape in tapes {
        if let Err(error) = demogod::Tape::parse(tape) {
            panic!("a README tape does not parse:\n{error}\n\n{tape}");
        }
    }
}

#[test]
fn every_tape_in_the_repository_parses() {
    for path in ["examples/hello.tape", "docs/demo/demo.tape", "docs/demo/hello.tape"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        if let Err(error) = demogod::Tape::from_file(&path) {
            panic!("{error}");
        }
    }
}
