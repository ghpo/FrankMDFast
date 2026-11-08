//! Explorer view-model tests.

use frankmd_workspace::explorer::*;

fn file(p: &str) -> Item {
    Item {
        path: p.into(),
        kind: ItemKind::File,
        encrypted: false,
    }
}
fn folder(p: &str) -> Item {
    Item {
        path: p.into(),
        kind: ItemKind::Folder,
        encrypted: false,
    }
}
fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        shift: false,
    }
}
fn shift() -> Modifiers {
    Modifiers {
        ctrl: false,
        shift: true,
    }
}
fn ctrl_shift() -> Modifiers {
    Modifiers {
        ctrl: true,
        shift: true,
    }
}
fn paths(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.path.as_str()).collect()
}
fn visible() -> Vec<Item> {
    vec![
        folder("docs"),
        file("docs/a.md"),
        file("b.md"),
        file("c.md"),
        file("d.md"),
    ]
}

#[test]
fn creation_destination() {
    // adapts: test/system/creation_destination_test.rb:35 "creating inside a collapsed selected folder opens the folder (#268)"
    let mut e = Explorer::default();
    assert_eq!(e.creation_folder(), "");
    e.click(folder("docs"), Modifiers::default(), &visible());
    assert_eq!(e.creation_folder(), "docs");
    e.click(folder("docs/nested"), Modifiers::default(), &[]);
    assert_eq!(e.creation_folder(), "docs/nested");
    e.click(file("b.md"), Modifiers::default(), &visible());
    assert_eq!(e.creation_folder(), "");
    e.select_only(folder("docs"));
    e.toggle_selected(file("b.md"));
    assert_eq!(e.creation_folder(), "");
}

#[test]
fn creation_reveal_expands_ancestors() {
    // adapts: test/system/creation_destination_test.rb:35 "creating inside a collapsed selected folder opens the folder (#268)"
    let mut e = Explorer::default();
    e.reveal("docs/nested/new.md");
    assert!(e.expanded().contains("docs"));
    assert!(e.expanded().contains("docs/nested"));
    assert!(!e.expanded().contains("docs/nested/new.md"));
    e.collapse_all();
    assert!(e.expanded().is_empty());
}

#[test]
fn plain_click_effects() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    let v = visible();
    assert_eq!(
        e.click(file("b.md"), Modifiers::default(), &v),
        Some(ExplorerEffect::OpenNote("b.md".into()))
    );
    assert_eq!(
        e.click(folder("docs"), Modifiers::default(), &v),
        Some(ExplorerEffect::ToggleFolder("docs".into()))
    );
    assert_eq!(paths(&e.selection()), ["docs"]);
    assert_eq!(e.cursor(), Some("docs"));
}

#[test]
fn ctrl_click_toggles_without_effect() {
    // adapts: test/system/notes_test.rb:373 "Ctrl selection deletes multiple notes from the context menu"
    let mut e = Explorer::default();
    let v = visible();
    e.click(file("b.md"), Modifiers::default(), &v);
    assert_eq!(e.click(file("c.md"), ctrl(), &v), None);
    assert_eq!(paths(&e.selection()), ["b.md", "c.md"]);
    e.click(file("b.md"), ctrl(), &v);
    assert_eq!(paths(&e.selection()), ["c.md"]);
}

#[test]
fn shift_click_selects_range_and_ctrl_shift_adds() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    let v = visible();
    e.click(file("docs/a.md"), Modifiers::default(), &v);
    assert_eq!(e.click(file("c.md"), shift(), &v), None);
    assert_eq!(paths(&e.selection()), ["docs/a.md", "b.md", "c.md"]);
    e.click(file("d.md"), ctrl(), &v);
    e.click(file("b.md"), shift(), &v);
    assert_eq!(paths(&e.selection()), ["b.md", "c.md", "d.md"]);
    e.click(folder("docs"), Modifiers::default(), &v);
    e.click(file("c.md"), ctrl(), &v);
    e.click(file("d.md"), ctrl_shift(), &v);
    assert_eq!(paths(&e.selection()), ["docs", "c.md", "d.md"]);
}

#[test]
fn shift_click_without_anchor_selects_only_item() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.click(file("c.md"), shift(), &visible());
    assert_eq!(paths(&e.selection()), ["c.md"]);
    e.select_only(file("gone.md"));
    e.click(file("b.md"), shift(), &visible());
    assert_eq!(paths(&e.selection()), ["b.md"]);
}

#[test]
fn select_all_and_clear() {
    // adapts: test/system/explorer_background_test.rb:6 "clicking the explorer's empty area deselects and offers the explorer menu (#261, #267)"
    let mut e = Explorer::default();
    let v = visible();
    e.select_only(file("b.md"));
    e.open_context_menu(MenuTarget::Background, 1.0, 2.0);
    e.click_background();
    assert!(e.selection().is_empty());
    assert_eq!(e.creation_folder(), "");
    assert!(e.context_menu().is_some());
    e.select_all(&v);
    assert_eq!(e.selection().len(), 5);
    assert_eq!(e.cursor(), Some("d.md"));
    e.extend_selection(file("b.md"), &v);
    assert_eq!(paths(&e.selection()), ["docs", "docs/a.md", "b.md"]);
}

#[test]
fn click_closes_context_menu() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.open_context_menu(MenuTarget::Row(file("b.md")), 0.0, 0.0);
    e.click(file("c.md"), Modifiers::default(), &visible());
    assert!(e.context_menu().is_none());
}

#[test]
fn right_click_reselects_unselected_row_but_keeps_selection() {
    // adapts: test/system/notes_test.rb:373 "Ctrl selection deletes multiple notes from the context menu"
    let mut e = Explorer::default();
    let v = visible();
    e.click(file("b.md"), Modifiers::default(), &v);
    e.click(file("c.md"), ctrl(), &v);
    e.open_context_menu(MenuTarget::Row(file("b.md")), 5.0, 6.0);
    assert_eq!(paths(&e.selection()), ["b.md", "c.md"]);
    assert_eq!(e.context_targets(&file("b.md")).len(), 2);
    e.open_context_menu(MenuTarget::Row(file("d.md")), 5.0, 6.0);
    assert_eq!(paths(&e.selection()), ["d.md"]);
    assert_eq!(e.context_menu().map(|m| (m.x, m.y)), Some((5.0, 6.0)));
    assert_eq!(paths(&e.context_targets(&file("b.md"))), ["b.md"]);
}

#[test]
fn no_context_menu_for_config_file() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.open_context_menu(MenuTarget::Row(file(".fed")), 0.0, 0.0);
    assert!(e.context_menu().is_none());
    e.open_context_menu(MenuTarget::Row(file("sub/.fed")), 0.0, 0.0);
    assert!(e.context_menu().is_some());
}

#[test]
fn menu_items_per_target() {
    // adapts: test/system/explorer_background_test.rb:6 "clicking the explorer's empty area deselects and offers the explorer menu (#261, #267)"
    use MenuItem::*;
    assert_eq!(
        Explorer::menu_items(&MenuTarget::Row(file("a.md"))),
        [Encrypt, Rename, MoveTo, Delete]
    );
    let mut encrypted = file("secret.md");
    encrypted.encrypted = true;
    assert_eq!(
        Explorer::menu_items(&MenuTarget::Row(encrypted)),
        [Decrypt, Rename, MoveTo, Delete]
    );
    assert_eq!(
        Explorer::menu_items(&MenuTarget::Row(folder("a"))),
        [NewNote, NewFolder, Rename, MoveTo, Delete]
    );
    assert_eq!(
        Explorer::menu_items(&MenuTarget::Background),
        [NewNote, NewFolder, SelectAll, FindFile, CollapseAll]
    );
}

#[test]
fn delete_targets_dedupe_and_drop_nested_and_config() {
    // adapts: test/system/notes_test.rb:373 "Ctrl selection deletes multiple notes from the context menu"
    let items = vec![
        file("docs/a.md"),
        folder("docs"),
        file("b.md"),
        file("b.md"),
        file(".fed"),
        file("docs-old/x.md"),
    ];
    assert_eq!(
        paths(&Explorer::delete_targets(&items)),
        ["docs", "b.md", "docs-old/x.md"]
    );
}

fn drag_setup() -> Explorer {
    let mut e = Explorer::default();
    let v = vec![
        file("first.md"),
        file("second.md"),
        file("rooted.md"),
        folder("target"),
    ];
    e.click(file("first.md"), Modifiers::default(), &v);
    e.click(file("second.md"), ctrl(), &v);
    e
}

#[test]
fn multi_drag_moves_whole_selection() {
    // adapts: test/system/multi_drag_test.rb:6 "dragging a selected group moves every selected item to the destination (#266)"
    let mut e = drag_setup();
    e.begin_drag(file("first.md"));
    assert!(e.is_dragging());
    assert_eq!(e.drag_badge_count(), 2);
    assert_eq!(paths(e.drag_payload()), ["first.md", "second.md"]);
    let plan = e.drop(&DropHover::Folder("target".into()));
    assert_eq!(
        plan,
        Some(MovePlan {
            items: vec!["first.md".into(), "second.md".into()],
            dest: "target".into()
        })
    );
    assert!(!e.is_dragging());
}

#[test]
fn dragging_unselected_item_moves_only_it() {
    // adapts: test/system/multi_drag_test.rb:47 "dragging an unselected item moves only that item (#266)"
    let mut e = drag_setup();
    e.begin_drag(file("rooted.md"));
    assert_eq!(e.drag_badge_count(), 1);
    let plan = e.drop(&DropHover::Folder("target".into()));
    assert_eq!(plan.map(|p| p.items), Some(vec!["rooted.md".to_string()]));
}

#[test]
fn config_file_cannot_be_dragged() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.begin_drag(file(".fed"));
    assert!(!e.is_dragging());
}

#[test]
fn drop_on_note_moves_into_its_folder() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.begin_drag(file("x.md"));
    assert_eq!(
        e.drop_target(&DropHover::File("docs/a.md".into())),
        Some(DropTarget::Folder("docs".into()))
    );
    // a note at the root: already at root, invalid
    assert_eq!(e.drop_target(&DropHover::File("b.md".into())), None);
    e.end_drag();
    e.begin_drag(file("docs/a.md"));
    assert_eq!(
        e.drop_target(&DropHover::File("b.md".into())),
        Some(DropTarget::Root)
    );
    assert_eq!(e.drop_target(&DropHover::File("docs/z.md".into())), None);
}

#[test]
fn drop_on_background_moves_to_root() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.begin_drag(file("docs/a.md"));
    let plan = e.drop(&DropHover::Background);
    assert_eq!(
        plan,
        Some(MovePlan {
            items: vec!["docs/a.md".into()],
            dest: String::new()
        })
    );
    e.begin_drag(file("b.md"));
    assert_eq!(e.drop_target(&DropHover::Background), None);
    assert_eq!(e.drop(&DropHover::Background), None);
    assert!(!e.is_dragging());
}

#[test]
fn invalid_drop_targets() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.begin_drag(folder("docs"));
    assert_eq!(e.drop_target(&DropHover::Folder("docs".into())), None);
    assert_eq!(e.drop_target(&DropHover::Folder("docs/sub".into())), None);
    assert_eq!(e.drop_target(&DropHover::File("docs/a.md".into())), None);
    assert_eq!(
        e.drop_target(&DropHover::Folder("docs-old".into())),
        Some(DropTarget::Folder("docs-old".into()))
    );
    e.end_drag();
    e.begin_drag(file("docs/a.md"));
    assert_eq!(e.drop_target(&DropHover::Folder("docs".into())), None);
}

#[test]
fn drop_skips_items_already_in_destination() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    let v = vec![file("t/in.md"), file("out.md"), folder("t")];
    e.click(file("t/in.md"), Modifiers::default(), &v);
    e.click(file("out.md"), ctrl(), &v);
    e.begin_drag(file("out.md"));
    let plan = e.drop(&DropHover::Folder("t".into()));
    assert_eq!(plan.map(|p| p.items), Some(vec!["out.md".to_string()]));
}

#[test]
fn remap_is_segment_wise() {
    // adapts: test/system/draft_recovery_test.rb:155 "remaps nested drafts and cleans them after deleting a folder"
    let mut e = Explorer::default();
    e.set_expanded(["docs".into(), "docs/sub".into(), "docs-old".into()]);
    e.select_only(file("docs/a.md"));
    e.toggle_selected(file("docs-old/x.md"));
    e.set_cursor(Some("docs/a.md".into()));
    e.remap("docs", "archive");
    assert_eq!(
        e.expanded().iter().map(String::as_str).collect::<Vec<_>>(),
        ["archive", "archive/sub", "docs-old"]
    );
    assert_eq!(paths(&e.selection()), ["archive/a.md", "docs-old/x.md"]);
    assert_eq!(e.cursor(), Some("archive/a.md"));
}

#[test]
fn remove_drops_descendants_only() {
    // adapts: test/system/draft_recovery_test.rb:155 "remaps nested drafts and cleans them after deleting a folder"
    let mut e = Explorer::default();
    e.set_expanded(["docs".into(), "docs/sub".into(), "docs-old".into()]);
    e.select_only(file("docs/a.md"));
    e.toggle_selected(file("docs-old/x.md"));
    e.set_cursor(Some("docs/a.md".into()));
    e.remove("docs");
    assert_eq!(
        e.expanded().iter().map(String::as_str).collect::<Vec<_>>(),
        ["docs-old"]
    );
    assert_eq!(paths(&e.selection()), ["docs-old/x.md"]);
    assert_eq!(e.cursor(), None);
}

#[test]
fn prune_drops_missing_entries() {
    // ui-spec §5.14
    let mut e = Explorer::default();
    e.select_only(file("a.md"));
    e.toggle_selected(file("b.md"));
    e.set_cursor(Some("a.md".into()));
    e.prune(|p| p == "b.md");
    assert_eq!(paths(&e.selection()), ["b.md"]);
    assert_eq!(e.cursor(), None);
}

#[test]
fn toggle_and_expand() {
    // adapts: test/system/notes_test.rb:191 "folders can be expanded and collapsed"
    let mut e = Explorer::default();
    e.toggle_folder("a");
    assert!(e.expanded().contains("a"));
    e.toggle_folder("a");
    assert!(!e.expanded().contains("a"));
    e.expand("a");
    e.collapse("a");
    assert!(e.expanded().is_empty());
}

#[test]
fn type_ahead_wraps_and_ignores_case_and_extension() {
    // ui-spec §5.14
    let v = vec![
        folder("Docs"),
        file("docs/alpha.md"),
        file("beta.md"),
        file("Bravo.md"),
    ];
    assert_eq!(type_ahead(&v, None, "b").as_deref(), Some("beta.md"));
    assert_eq!(
        type_ahead(&v, Some("beta.md"), "b").as_deref(),
        Some("Bravo.md")
    );
    assert_eq!(
        type_ahead(&v, Some("Bravo.md"), "b").as_deref(),
        Some("beta.md")
    );
    assert_eq!(
        type_ahead(&v, Some("beta.md"), "al").as_deref(),
        Some("docs/alpha.md")
    );
    assert_eq!(type_ahead(&v, None, "md"), None);
    assert_eq!(type_ahead(&[], None, "a"), None);
}
