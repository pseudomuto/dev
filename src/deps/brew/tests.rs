use super::*;

#[test]
fn parse_version_reads_clap_output() {
    assert_eq!(Some("0.4.2"), parse_version("zb 0.4.2\n"));
    assert_eq!(None, parse_version("something else 0.4.2"));
    assert_eq!(None, parse_version("zb \n"));
}

#[test]
fn parse_release_tag_reads_redirect_url() {
    let url = "https://github.com/zerobrewhq/zerobrew/releases/tag/v0.4.2";
    assert_eq!(Some("0.4.2"), parse_release_tag(url));
    assert_eq!(Some("0.4.2"), parse_release_tag(".../releases/tag/0.4.2"));
    assert_eq!(None, parse_release_tag(""));
    assert_eq!(None, parse_release_tag(".../releases/tag/v"));
}
