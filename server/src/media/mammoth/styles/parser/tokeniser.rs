//! mammoth `lib/styles/parser/tokeniser.js` over lop's `RegexTokeniser`: at
//! each position the first rule that matches there (non-empty) wins,
//! otherwise one `unrecognisedCharacter`.

use std::sync::LazyLock;

use regex::Regex;

use super::super::super::js::{JS_DOT_CLASS, JS_WHITESPACE_CLASS};
use super::super::super::lop::{Token, TokenValue};

struct TokenRule {
    name: &'static str,
    regex: Regex,
    /// Whether the regex has a group (the token's value).
    grouped: bool,
}

static RULES: LazyLock<Vec<TokenRule>> = LazyLock::new(|| {
    let escape = format!(r"\\{JS_DOT_CLASS}");
    let identifier_character = format!(r"(?:[a-zA-Z\-_]|{escape})");
    let string_prefix = format!(r"'((?:{escape}|[^'])*)");
    let rules: Vec<(&'static str, String, bool)> = vec![
        ("identifier", format!("({identifier_character}(?:{identifier_character}|[0-9])*)"), true),
        ("dot", r"\.".into(), false),
        ("colon", ":".into(), false),
        ("gt", ">".into(), false),
        ("whitespace", format!("[{JS_WHITESPACE_CLASS}]+"), false),
        ("arrow", "=>".into(), false),
        ("equals", "=".into(), false),
        ("startsWith", r"\^=".into(), false),
        ("open-paren", r"\(".into(), false),
        ("close-paren", r"\)".into(), false),
        ("open-square-bracket", r"\[".into(), false),
        ("close-square-bracket", r"\]".into(), false),
        ("string", format!("{string_prefix}'"), true),
        ("unterminated-string", string_prefix.clone(), true),
        ("integer", "([0-9]+)".into(), true),
        ("choice", r"\|".into(), false),
        ("bang", "(!)".into(), true),
    ];
    rules
        .into_iter()
        .map(|(name, pattern, grouped)| TokenRule {
            name,
            regex: Regex::new(&format!("^(?:{pattern})")).expect("tokeniser regex"),
            grouped,
        })
        .collect()
});

/// `tokenise(string)`.
pub fn tokenise(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0;
    'outer: while index < input.len() {
        let rest = &input[index..];
        for rule in RULES.iter() {
            if let Some(caps) = rule.regex.captures(rest) {
                let whole = caps.get(0).expect("match");
                if whole.end() > 0 {
                    let value = if rule.grouped {
                        TokenValue::Str(caps.get(1).map_or(String::new(), |m| m.as_str().to_owned()))
                    } else {
                        TokenValue::Undefined
                    };
                    tokens.push(Token { name: rule.name, value });
                    index += whole.end();
                    continue 'outer;
                }
            }
        }
        let c = rest.chars().next().expect("non-empty");
        tokens.push(Token { name: "unrecognisedCharacter", value: TokenValue::Str(c.to_string()) });
        index += c.len_utf8();
    }
    tokens.push(Token { name: "end", value: TokenValue::Null });
    tokens
}
