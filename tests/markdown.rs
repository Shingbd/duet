//! Display-only regressions; human-started with `cargo test --test markdown`.
//! These tests do not invoke a model, renderer process, or network.
use duet_workbench::{
    markdown::{self, Block, Context, Fragment, InlineStyle, Role, SemanticLine, Status},
    ui::style_markdown,
};
use ratatui::style::Modifier;

fn text(lines: &[SemanticLine]) -> String {
    lines
        .iter()
        .map(SemanticLine::text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn supported_blocks_inline_styles_and_literal_code_are_explicit() {
    let source = "# Heading\n\n**bold** *italic* ~~strike~~ `inline_*`\n\n> quoted\n\n1. outer\n   - nested\n\n- [ ] pending document\n- [x] checked document\n\n---\n\n```rust\n# literal\n*a_code*\n\n```\n\n    indented_code\n\n[link](https://example.test) ![alt](image.png)";
    let doc = markdown::parse(source);
    assert!(doc.blocks.iter().any(|block| matches!(block, Block::Rule)));
    let lines = doc.layout(100, Context::default());
    let fragments: Vec<_> = lines.iter().flat_map(|line| &line.fragments).collect();
    for predicate in [
        ("heading", fragments.iter().any(|f| f.style.heading == 1)),
        ("strong", fragments.iter().any(|f| f.style.strong)),
        ("emphasis", fragments.iter().any(|f| f.style.emphasis)),
        ("strike", fragments.iter().any(|f| f.style.strike)),
        ("quote", fragments.iter().any(|f| f.style.quote)),
        ("link", fragments.iter().any(|f| f.style.link)),
        (
            "code",
            fragments
                .iter()
                .any(|f| f.style.code && f.text.contains("*a_code*")),
        ),
    ] {
        assert!(predicate.1, "missing {} style", predicate.0);
    }
    let output = text(&lines);
    for expected in [
        "> quoted",
        "1. outer",
        "• nested",
        "[ ] pending document",
        "[x] checked document",
        "# literal",
        "indented_code",
        "https://example.test",
        "[图片: alt] (image.png)",
    ] {
        assert!(output.contains(expected), "missing {expected}");
    }
    assert!(lines.iter().all(|line| line.context.status == Status::None));
    for light in [false, true] {
        let styled = style_markdown(&lines, light);
        assert!(
            styled
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            styled
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }
}

#[test]
fn hostile_controls_html_entities_and_unsupported_markup_stay_readable() {
    for source in [
        "<script>alert('literal')</script>",
        "before\x1b[2Jafter\u{009b}31m\rEND",
        "\x1b]8;;https://example.test\x07label\x1b]8;;\x07",
        "\x1b]52;c;c2VjcmV0\x07clipboard\0end",
        "[unsafe](javascript:alert%281%29) ![remote](https://example.test/image.png)",
        "&#27;[31m &#9;tab",
        "unclosed **emphasis",
        "```rust\n*a_literal*\n# still code",
        "$$ unsupported math $$",
    ] {
        let output = text(&markdown::render(source, 80, Context::default()));
        assert!(!output.is_empty());
        assert!(output.chars().all(|ch| ch == '\n' || !ch.is_control()));
    }
    let fallback = text(&markdown::render(
        "[unsafe](javascript:alert%281%29) ![remote](https://example.test/image.png)",
        100,
        Context::default(),
    ));
    assert!(fallback.contains("unsafe") && fallback.contains("javascript:"));
    assert!(fallback.contains("remote") && fallback.contains("https://example.test/image.png"));
    assert!(
        text(&markdown::render("<b>literal</b>", 80, Context::default()))
            .contains("<b>literal</b>")
    );
    assert!(
        text(&markdown::render(
            "unclosed **emphasis",
            80,
            Context::default()
        ))
        .contains("**emphasis")
    );
    assert!(
        text(&markdown::render(
            "```\n*a_literal*",
            80,
            Context::default()
        ))
        .contains("*a_literal*")
    );
    let fragments = [Fragment {
        text: "x\x1b[2J\t\u{009b}".into(),
        style: InlineStyle::default(),
    }];
    assert!(
        markdown::wrap(&fragments, 4, Context::default())
            .iter()
            .all(|line| line.text().chars().all(|ch| !ch.is_control()))
    );
}

#[test]
fn unicode_wraps_across_fragment_styles_and_preserves_display_context() {
    let fragments = [
        Fragment {
            text: "e".into(),
            style: InlineStyle {
                strong: true,
                ..Default::default()
            },
        },
        Fragment {
            text: "\u{301}中文👩‍💻**source**".into(),
            style: InlineStyle::default(),
        },
    ];
    let context = Context {
        role: Role::Assistant,
        status: Status::Pending,
    };
    for width in [2, 3, 8, 80] {
        let lines = markdown::wrap(&fragments, width, context);
        assert_eq!(
            lines.iter().map(SemanticLine::text).collect::<String>(),
            "e\u{301}中文👩‍💻**source**"
        );
        assert!(
            lines
                .iter()
                .all(|line| line.width() <= width && line.context == context)
        );
        assert!(lines.iter().any(|line| line.text().contains("e\u{301}")));
        assert!(lines.iter().any(|line| line.text().contains("👩‍💻")));
    }
    assert!(markdown::render("中", 0, context).is_empty());
    assert_eq!(text(&markdown::render("中", 1, context)), "?");
}

#[test]
fn narrow_tables_keep_every_value_and_layout_is_repeatable() {
    let source = "| Name | Value |\n| --- | --- |\n| 中文 | **long value** |\n| emoji | `👩‍💻` |";
    let document = markdown::parse(source);
    assert!(
        document
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Table { .. }))
    );
    let wide = document.layout(80, Context::default());
    assert!(text(&wide).contains(" │ "));
    for width in [2, 8, 12, 24] {
        let lines = document.layout(width, Context::default());
        assert!(lines.iter().all(|line| line.width() <= width));
        let flattened: String = lines.iter().map(SemanticLine::text).collect();
        for value in ["中文", "long value", "emoji", "👩‍💻"] {
            assert!(flattened.contains(value), "lost {value} at width {width}");
        }
    }
    assert_eq!(document.layout(80, Context::default()), wide);
    assert!(source.contains("**long value**"));
}

#[test]
fn growing_stream_reparses_whole_message_without_losing_fenced_blank_lines() {
    for source in [
        "# Partial",
        "# Partial\n\n```\none\n",
        "# Partial\n\n```\none\n\n```\n\n**final**",
    ] {
        let lines = markdown::render(source, 24, Context::default());
        assert!(text(&lines).contains("Partial"));
        assert!(lines.iter().all(|line| line.width() <= 24));
    }
    let lines = markdown::render("```\none\n\n```", 24, Context::default());
    assert_eq!(
        lines.iter().map(SemanticLine::text).collect::<Vec<_>>(),
        ["one", ""]
    );
}
