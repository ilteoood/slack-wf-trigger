pub fn strip(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '<' => {
                let mut inner = String::new();
                let mut closed = false;
                for nc in chars.by_ref() {
                    if nc == '>' {
                        closed = true;
                        break;
                    }
                    inner.push(nc);
                }
                if closed {
                    push_special(&mut out, &inner);
                } else {
                    out.push('<');
                    out.push_str(&inner);
                }
            }
            '*' | '_' | '~' | '`' => {}
            _ => out.push(c),
        }
    }

    out
}

fn push_special(out: &mut String, inner: &str) {
    if let Some(rest) = inner.strip_prefix('@') {
        if let Some((_, display)) = rest.split_once('|') {
            out.push('@');
            out.push_str(display);
        } else {
            out.push('@');
            out.push_str(rest);
        }
        return;
    }

    if let Some(rest) = inner.strip_prefix('#') {
        if let Some((_, display)) = rest.split_once('|') {
            out.push('#');
            out.push_str(display);
        } else {
            out.push('#');
            out.push_str(rest);
        }
        return;
    }

    if let Some(rest) = inner.strip_prefix('!') {
        if let Some((_, display)) = rest.split_once('|') {
            out.push_str(display);
        } else {
            out.push('@');
            out.push_str(rest.trim_start_matches("subteam^"));
        }
        return;
    }

    if let Some((_, display)) = inner.split_once('|') {
        out.push_str(display);
    } else {
        out.push_str(inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_link_with_display_text() {
        assert_eq!(strip("<https://x.com|deploy now>"), "deploy now");
    }

    #[test]
    fn keeps_bare_url() {
        assert_eq!(strip("<https://x.com>"), "https://x.com");
    }

    #[test]
    fn strips_user_mention() {
        assert_eq!(strip("hi <@U0123>"), "hi @U0123");
    }

    #[test]
    fn strips_user_mention_with_alias() {
        assert_eq!(strip("hi <@U0123|alice>"), "hi @alice");
    }

    #[test]
    fn strips_channel_reference() {
        assert_eq!(strip("see <#C0123|general>"), "see #general");
    }

    #[test]
    fn translates_special_mentions() {
        assert_eq!(strip("<!here>"), "@here");
        assert_eq!(strip("<!channel>"), "@channel");
        assert_eq!(strip("<!everyone>"), "@everyone");
        assert_eq!(strip("<!subteam^S0123>"), "@S0123");
        assert_eq!(strip("<!subteam^S0123|@team>"), "@team");
    }

    #[test]
    fn strips_basic_formatting_chars() {
        assert_eq!(strip("*bold* and _italic_"), "bold and italic");
        assert_eq!(strip("`inline code`"), "inline code");
        assert_eq!(strip("~strike~"), "strike");
    }

    #[test]
    fn leaves_unclosed_angle_bracket_alone() {
        assert_eq!(strip("a < b and <@U1"), "a < b and <@U1");
    }

    #[test]
    fn combined_message() {
        let raw = "Hey <@U1|bob>, see <https://x.com|deploy> in <#C1|general> — *now*!";
        assert_eq!(strip(raw), "Hey @bob, see deploy in #general — now!");
    }
}
