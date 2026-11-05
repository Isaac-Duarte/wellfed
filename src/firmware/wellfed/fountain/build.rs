//! Forwards the board `wellfed-board` chose: the boot test is the fountain's,
//! and builds to nothing on any other board.

#[path = "../../../sdk/wellfed-board/build/forward.rs"]
mod board;

fn main() {
    board::forward();
}
