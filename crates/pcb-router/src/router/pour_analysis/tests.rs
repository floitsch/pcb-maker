// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use super::joins::{Joins, main_piece};
use super::*;

#[test]
fn joins_lay_out_pieces_then_terminals_then_branches() {
    let joins = Joins::new(3, 2, 4);
    assert_eq!(joins.terminal_base, 4);
    assert_eq!(joins.branch_base, 6);
    assert_eq!(joins.parent, (0..10).collect::<Vec<_>>());
}

#[test]
fn the_main_piece_holds_the_most_terminals() {
    // Pieces 1 and 2; terminals 0 to 2 at 3 to 5.
    let mut joins = Joins::new(2, 3, 0);
    joins.union(3, 1);
    joins.union(4, 2);
    joins.union(5, 2);
    let labels = vec![vec![1, 1, 1, 1, 2]];
    let main = main_piece(&mut joins.parent, &labels, joins.terminal_base, 3);
    assert_eq!(main, find(&mut joins.parent, 2));
}

#[test]
fn on_equal_votes_the_main_piece_has_the_most_fill() {
    let mut joins = Joins::new(2, 2, 0);
    joins.union(3, 1);
    joins.union(4, 2);
    // Piece 2 has more nodes than piece 1.
    let labels = vec![vec![1, 2, 0], vec![2, 2, 0]];
    let main = main_piece(&mut joins.parent, &labels, joins.terminal_base, 2);
    assert_eq!(main, find(&mut joins.parent, 2));
    // Equal fill too: the least root.
    let mut joins = Joins::new(2, 2, 0);
    joins.union(3, 1);
    joins.union(4, 2);
    let labels = vec![vec![1, 2]];
    let main = main_piece(&mut joins.parent, &labels, joins.terminal_base, 2);
    assert_eq!(main, 1);
}

#[test]
fn without_terminals_the_main_piece_is_zero() {
    let mut joins = Joins::new(2, 0, 1);
    assert_eq!(main_piece(&mut joins.parent, &[vec![1, 2]], 3, 0), 0);
}
