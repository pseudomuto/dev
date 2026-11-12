use super::*;

#[test]
fn tail_message_appends_the_latest_line() {
    assert_eq!(
        "Installing jq · ==> Downloading",
        tail_message("Installing jq", "==> Downloading\n", 80)
    );
}

#[test]
fn tail_message_is_just_the_name_for_blank_lines() {
    assert_eq!("Installing jq", tail_message("Installing jq", "   ", 80));
}

#[test]
fn tail_message_strips_colors() {
    assert_eq!(
        "Installing jq · installed",
        tail_message("Installing jq", "\u{1b}[32minstalled\u{1b}[0m", 80)
    );
}

#[test]
fn tail_message_fits_in_the_terminal_after_the_rail() {
    let msg = tail_message("Installing jq", &"x".repeat(100), 30);
    assert_eq!(30 - MESSAGE_INDENT, console::measure_text_width(&msg), "msg: {msg}");
    assert!(msg.ends_with('…'), "msg: {msg}");
}

#[test]
fn error_lines_show_each_cause_then_its_output_then_the_log() {
    let causes = vec![
        "task failed: Installing jq".to_string(),
        "`zb` failed with exit status: 1. Last output:\n    ==> Installing jq\n    Error: no bottle".to_string(),
    ];
    assert_eq!(
        vec![
            "task failed: Installing jq",
            "`zb` failed with exit status: 1. Last output:",
            "",
            "    ==> Installing jq",
            "    Error: no bottle",
            "",
            "Full output: .dev/log/1-up.log",
        ],
        error_lines(&causes, ".dev/log/1-up.log", 80)
    );
}

#[test]
fn error_lines_wrap_inside_the_rail_keeping_the_indent() {
    let causes = vec!["cause:\n    one two three four five six".to_string()];
    // 3 columns go to the rail (`│  `), leaving 20.
    assert_eq!(
        vec![
            "cause:",
            "",
            "    one two three",
            "      four five six",
            "",
            "Full output: log"
        ],
        error_lines(&causes, "log", 23)
    );
}

#[test]
fn error_lines_strip_colors_from_output() {
    let causes = vec!["cause:\n    \u{1b}[31mred\u{1b}[0m".to_string()];
    assert_eq!(
        vec!["cause:", "", "    red", "", "Full output: log"],
        error_lines(&causes, "log", 80)
    );
}
