//! lop 0.4.2, the parser-combinator library mammoth's style-map reader is
//! written in: `token`, `firstOf`, `then`, `sequence` (with `capture`, `cut`,
//! `head`, `map`), `optional`, `zeroOrMore` and the separated repetitions,
//! over a token list ending in an `end` token.
//!
//! Ported with its quirks, because they decide which style-map lines parse:
//! a success whose value is `undefined` is never mapped; `head()` returns the
//! LAST unnamed capture (they all share the key "undefined"); after a `cut`
//! a failure becomes an error that stops `firstOf`; and an error inside the
//! repeated part of a separated repetition turns into a success whose list
//! ends in `undefined`.

use std::collections::HashMap;
use std::rc::Rc;

/// A token (`Token`): `value` is the regex's first group, `None` when the
/// rule has none (`undefined`); the `end` token's value is `null`.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub name: &'static str,
    pub value: TokenValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenValue {
    Undefined,
    Null,
    Str(String),
}

/// `TokenIterator`.
#[derive(Debug, Clone)]
pub struct TokenIterator {
    tokens: Rc<Vec<Token>>,
    start: usize,
}

impl TokenIterator {
    pub fn new(tokens: Vec<Token>) -> Self {
        TokenIterator { tokens: Rc::new(tokens), start: 0 }
    }

    fn head(&self) -> Option<&Token> {
        self.tokens.get(self.start)
    }

    fn tail(&self) -> TokenIterator {
        TokenIterator { tokens: Rc::clone(&self.tokens), start: self.start + 1 }
    }
}

/// A parse value: what JavaScript would hold. `D` is the grammar's own
/// objects.
#[derive(Debug, Clone)]
pub enum Value<D> {
    Undefined,
    Null,
    Str(String),
    Array(Vec<Value<D>>),
    /// The `option` package: `Some(v)` / `none`.
    Opt(Option<Box<Value<D>>>),
    Seq(Rc<SequenceValues<D>>),
    /// The `☃source☃` capture every sequence adds (never looked at here).
    Source,
    Dom(D),
}

impl<D> Value<D> {
    fn has_value(&self) -> bool {
        !matches!(self, Value::Undefined)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Success,
    Failure,
    Error,
    Cut,
}

/// A parse result (`Result` of parsing-results.js; the error list is not
/// kept, only that there is one).
#[derive(Debug, Clone)]
pub struct PResult<D> {
    pub status: Status,
    pub value: Value<D>,
    pub remaining: TokenIterator,
}

impl<D: Clone> PResult<D> {
    fn success(value: Value<D>, remaining: TokenIterator) -> Self {
        PResult { status: Status::Success, value, remaining }
    }

    fn failure(remaining: TokenIterator) -> Self {
        PResult { status: Status::Failure, value: Value::Undefined, remaining }
    }

    fn error(remaining: TokenIterator) -> Self {
        PResult { status: Status::Error, value: Value::Undefined, remaining }
    }

    pub fn is_success(&self) -> bool {
        matches!(self.status, Status::Success | Status::Cut)
    }

    fn is_error(&self) -> bool {
        self.status == Status::Error
    }

    /// `result.map(func)`: only when there is a value.
    fn map(self, func: impl FnOnce(Value<D>) -> Value<D>) -> Self {
        if self.value.has_value() {
            PResult { status: self.status, value: func(self.value), remaining: self.remaining }
        } else {
            self
        }
    }
}

/// `SequenceValues`.
#[derive(Debug, Clone)]
pub struct SequenceValues<D> {
    values: HashMap<String, Value<D>>,
    array: Vec<Value<D>>,
}

impl<D: Clone> SequenceValues<D> {
    fn new() -> Self {
        SequenceValues { values: HashMap::new(), array: Vec::new() }
    }

    fn with_value(&self, key: &str, value: Value<D>) -> Self {
        let mut values = self.values.clone();
        values.insert(key.to_owned(), value.clone());
        let mut array = self.array.clone();
        array.push(value);
        SequenceValues { values, array }
    }

    fn get(&self, key: &str) -> Value<D> {
        self.values.get(key).cloned().unwrap_or(Value::Undefined)
    }

    pub fn to_array(&self) -> &[Value<D>] {
        &self.array
    }
}

type RuleFn<D> = Rc<dyn Fn(&TokenIterator) -> PResult<D>>;

/// A rule, with lop's `captureName` / `isCaptured` properties.
pub struct Rule<D> {
    f: RuleFn<D>,
    captured: bool,
    /// `None` is the key "undefined".
    capture_name: Option<&'static str>,
}

impl<D> Clone for Rule<D> {
    fn clone(&self) -> Self {
        Rule { f: Rc::clone(&self.f), captured: self.captured, capture_name: self.capture_name }
    }
}

impl<D: Clone + 'static> Rule<D> {
    fn new(f: impl Fn(&TokenIterator) -> PResult<D> + 'static) -> Self {
        Rule { f: Rc::new(f), captured: false, capture_name: None }
    }

    pub fn parse(&self, input: &TokenIterator) -> PResult<D> {
        (self.f)(input)
    }

    fn key(&self) -> &'static str {
        self.capture_name.unwrap_or("undefined")
    }
}

/// `token(tokenType, value)`.
pub fn token<D: Clone + 'static>(token_type: &'static str, value: Option<&'static str>) -> Rule<D> {
    Rule::new(move |input| match input.head() {
        Some(t) if t.name == token_type && value.is_none_or(|v| t.value == TokenValue::Str(v.to_owned())) => {
            let v = match &t.value {
                TokenValue::Undefined => Value::Undefined,
                TokenValue::Null => Value::Null,
                TokenValue::Str(s) => Value::Str(s.clone()),
            };
            PResult::success(v, input.tail())
        }
        _ => PResult::failure(input.clone()),
    })
}

/// `tokenOfType(tokenType)`.
pub fn token_of_type<D: Clone + 'static>(token_type: &'static str) -> Rule<D> {
    token(token_type, None)
}

/// `firstOf(name, ...parsers)`.
pub fn first_of<D: Clone + 'static>(parsers: Vec<Rule<D>>) -> Rule<D> {
    Rule::new(move |input| {
        for p in &parsers {
            let r = p.parse(input);
            if r.is_success() || r.is_error() {
                return r;
            }
        }
        PResult::failure(input.clone())
    })
}

/// `then(parser, func)`.
pub fn then<D: Clone + 'static>(parser: Rule<D>, func: impl Fn(Value<D>) -> Value<D> + 'static) -> Rule<D> {
    let func = Rc::new(func);
    Rule::new(move |input| {
        let f = Rc::clone(&func);
        parser.parse(input).map(move |v| f(v))
    })
}

/// `sequence(...parsers)`.
pub fn sequence<D: Clone + 'static>(parsers: Vec<Rule<D>>) -> Rule<D> {
    Rule::new(move |input| {
        let mut result = PResult::success(Value::Seq(Rc::new(SequenceValues::new())), input.clone());
        let mut has_cut = false;
        for parser in &parsers {
            if !result.is_success() {
                continue;
            }
            let sub = parser.parse(&result.remaining);
            if sub.status == Status::Cut {
                has_cut = true;
            } else if sub.is_success() {
                let values = match &result.value {
                    Value::Seq(v) => {
                        if parser.captured {
                            Rc::new(v.with_value(parser.key(), sub.value))
                        } else {
                            Rc::clone(v)
                        }
                    }
                    _ => unreachable!("sequence values"),
                };
                result = PResult::success(Value::Seq(values), sub.remaining);
            } else if has_cut {
                result = PResult::error(sub.remaining);
            } else {
                result = sub;
            }
        }
        result.map(|v| match v {
            Value::Seq(values) => Value::Seq(Rc::new(values.with_value("☃source☃", Value::Source))),
            other => other,
        })
    })
}

/// `sequence.capture(rule, name)`.
pub fn capture<D: Clone + 'static>(rule: Rule<D>, name: Option<&'static str>) -> Rule<D> {
    Rule { f: rule.f, captured: true, capture_name: name }
}

/// `sequence.cut()`.
pub fn cut<D: Clone + 'static>() -> Rule<D> {
    Rule::new(|input| PResult { status: Status::Cut, value: Value::Undefined, remaining: input.clone() })
}

/// `rule.head()` of a sequence built from `parsers` (the value of the first
/// captured rule's key).
pub fn head<D: Clone + 'static>(parsers: Vec<Rule<D>>) -> Rule<D> {
    let key = parsers.iter().find(|p| p.captured).map(|p| p.key()).unwrap_or("undefined");
    then(sequence(parsers), move |v| match v {
        Value::Seq(values) => values.get(key),
        other => other,
    })
}

/// `rule.map(func)` of a sequence: `func` gets every captured value (and the
/// source, last).
pub fn map<D: Clone + 'static>(parsers: Vec<Rule<D>>, func: impl Fn(&[Value<D>]) -> Value<D> + 'static) -> Rule<D> {
    then(sequence(parsers), move |v| match v {
        Value::Seq(values) => func(values.to_array()),
        other => other,
    })
}

/// `optional(rule)`.
pub fn optional<D: Clone + 'static>(rule: Rule<D>) -> Rule<D> {
    Rule::new(move |input| {
        let result = rule.parse(input);
        if result.is_success() {
            result.map(|v| Value::Opt(Some(Box::new(v))))
        } else if result.status == Status::Failure {
            PResult::success(Value::Opt(None), input.clone())
        } else {
            result
        }
    })
}

/// `zeroOrMore(rule)`.
pub fn zero_or_more<D: Clone + 'static>(rule: Rule<D>) -> Rule<D> {
    Rule::new(move |input| {
        let mut input = input.clone();
        let mut values = Vec::new();
        loop {
            let result = rule.parse(&input);
            if !result.is_success() {
                if result.is_error() {
                    return result;
                }
                return PResult::success(Value::Array(values), input);
            }
            input = result.remaining.clone();
            values.push(result.value);
        }
    })
}

fn repeated_with_separator<D: Clone + 'static>(rule: Rule<D>, separator: Rule<D>, one_or_more: bool) -> Rule<D> {
    Rule::new(move |input| {
        let result = rule.parse(input);
        if result.is_success() {
            let main = capture(rule.clone(), Some("main"));
            let remaining_rule = zero_or_more(then(sequence(vec![separator.clone(), main]), |v| match v {
                Value::Seq(values) => values.get("main"),
                other => other,
            }));
            let remaining = remaining_rule.parse(&result.remaining);
            let mut list = vec![result.value];
            // `[first].concat(remaining.value())`: an error's undefined is
            // appended as an element.
            match remaining.value {
                Value::Array(rest) => list.extend(rest),
                other => list.push(other),
            }
            PResult::success(Value::Array(list), remaining.remaining)
        } else if one_or_more || result.is_error() {
            result
        } else {
            PResult::success(Value::Array(Vec::new()), input.clone())
        }
    })
}

/// `zeroOrMoreWithSeparator(rule, separator)`.
pub fn zero_or_more_with_separator<D: Clone + 'static>(rule: Rule<D>, separator: Rule<D>) -> Rule<D> {
    repeated_with_separator(rule, separator, false)
}

/// `oneOrMoreWithSeparator(rule, separator)`.
pub fn one_or_more_with_separator<D: Clone + 'static>(rule: Rule<D>, separator: Rule<D>) -> Rule<D> {
    repeated_with_separator(rule, separator, true)
}
