//! Status changes reach the rows: nothing plays on the first load, a CI turning red does on the next rebuild.

mod common;

use common::{app, inputs};

#[test]
fn nothing_plays_on_the_first_load_and_a_change_does() {
    let mut app = app(false);
    assert!(!app.playing());
    app.feed(inputs());
    assert!(!app.playing(), "the same data again changes nothing");
    let mut next = inputs();
    let mine = next.data.as_mut().and_then(|d| d["mine"].as_array_mut()).expect("my PRs");
    mine[0]["ci"] = "FAILURE".into();
    app.feed(next);
    assert!(app.playing());
}
