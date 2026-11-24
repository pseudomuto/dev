use super::*;

fn lines(lines: &[&str]) -> VecDeque<String> {
    lines.iter().map(ToString::to_string).collect()
}

#[test]
fn output_message_is_just_the_name_without_output() {
    assert_eq!("Installing jq", output_message("Installing jq", &lines(&[]), 80));
}

#[test]
fn output_message_puts_each_line_under_the_name() {
    let msg = output_message("Installing jq", &lines(&["==> Downloading", "==> Pouring"]), 80);
    assert_eq!(
        "Installing jq\n│      ==> Downloading\n│      ==> Pouring",
        strip_ansi_codes(&msg)
    );
}

#[test]
fn output_message_strips_colors() {
    let msg = output_message("Installing jq", &lines(&["\u{1b}[32minstalled\u{1b}[0m"]), 80);
    assert_eq!("Installing jq\n│      installed", strip_ansi_codes(&msg));
}

#[test]
fn output_message_fits_each_line_in_the_terminal() {
    let msg = output_message(&"n".repeat(100), &lines(&[&"x".repeat(100)]), 30);
    let msg = strip_ansi_codes(&msg);
    let shown: Vec<&str> = msg.lines().collect();
    assert_eq!(2, shown.len(), "msg: {msg}");
    // The name follows the spinner, so it gets what's left after it.
    assert_eq!(30 - MESSAGE_INDENT, console::measure_text_width(shown[0]), "msg: {msg}");
    assert_eq!(30, console::measure_text_width(shown[1]), "msg: {msg}");
    assert!(shown.iter().all(|l| l.ends_with('…')), "msg: {msg}");
}

#[test]
fn push_output_keeps_the_last_non_blank_lines() {
    let mut output = VecDeque::new();
    for line in ["1", "  ", "2", "\u{1b}[31m3\u{1b}[0m", "4  "] {
        push_output(&mut output, line);
    }
    assert_eq!(lines(&["2", "3", "4"]), output);
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
