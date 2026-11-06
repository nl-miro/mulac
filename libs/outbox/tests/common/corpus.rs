//! Payload corpus shared by the validation suite and the serde feature matrix.
#![allow(dead_code)]

/// A payload with exactly `total` nested containers, counting the outermost.
pub fn nested(total: usize) -> String {
    let is_object = |i: usize| i.is_multiple_of(2);
    let mut s = if is_object(total - 1) { "{}" } else { "[]" }.to_string();
    for i in (0..total - 1).rev() {
        s = if is_object(i) {
            format!(r#"{{"k":{s}}}"#)
        } else {
            format!("[{s}]")
        };
    }
    s
}

pub fn rejected() -> Vec<(&'static str, String)> {
    let too_deep = nested(128);
    vec![
        ("array", "[]".into()),
        ("null", "null".into()),
        ("number", "1".into()),
        ("string", r#""s""#.into()),
        ("true", "true".into()),
        ("empty", String::new()),
        ("whitespace only", " \n\t".into()),
        ("two values", "{} {}".into()),
        ("trailing text", "{}x".into()),
        ("BOM", "\u{feff}{}".into()),
        ("NBSP", "\u{a0}{}".into()),
        ("form feed", "\u{c}{}".into()),
        ("vertical tab", "\u{b}{}".into()),
        ("trailing comma", r#"{"a":1,}"#.into()),
        ("leading comma", r#"{,"a":1}"#.into()),
        ("single quotes", "{'a':1}".into()),
        ("unquoted key", "{a:1}".into()),
        ("comment", r#"{"a":1/*x*/}"#.into()),
        ("duplicate key", r#"{"a":1,"a":2}"#.into()),
        ("escaped duplicate key", r#"{"a":1,"\u0061":2}"#.into()),
        (
            "escaped duplicate key, escape first",
            r#"{"\u0061":1,"a":2}"#.into(),
        ),
        (
            "two escapes of one key",
            r#"{"\u0061":1,"\u0061":2}"#.into(),
        ),
        ("duplicate nested", r#"{"o":{"a":1,"a":1}}"#.into()),
        ("duplicate across escapes", r#"{"é":1,"\u00e9":2}"#.into()),
        (
            "duplicate across hex case",
            r#"{"\u00e9":1,"\u00E9":2}"#.into(),
        ),
        (
            "duplicate across surrogate pair",
            r#"{"😀":1,"\ud83d\ude00":2}"#.into(),
        ),
        (
            "duplicate across solidus escape",
            r#"{"a/b":1,"a\/b":2}"#.into(),
        ),
        (
            "escaped duplicate nested",
            r#"{"o":{"\u0061":1,"a":1}}"#.into(),
        ),
        ("lone high surrogate", r#"{"s":"\ud800"}"#.into()),
        ("lone low surrogate", r#"{"s":"\udc00"}"#.into()),
        ("high then non-low", r#"{"s":"\ud800a"}"#.into()),
        ("reversed pair", r#"{"s":"\udc00\ud800"}"#.into()),
        ("lone surrogate key", r#"{"\ud800":1}"#.into()),
        ("leading zero", r#"{"n":01}"#.into()),
        ("trailing dot", r#"{"n":1.}"#.into()),
        ("leading dot", r#"{"n":.5}"#.into()),
        ("plus sign", r#"{"n":+1}"#.into()),
        ("bare exponent", r#"{"n":1e}"#.into()),
        ("signed bare exponent", r#"{"n":1e+}"#.into()),
        ("lone minus", r#"{"n":-}"#.into()),
        ("double minus", r#"{"n":--1}"#.into()),
        ("hex", r#"{"n":0x10}"#.into()),
        ("NaN", r#"{"n":NaN}"#.into()),
        ("Infinity", r#"{"n":Infinity}"#.into()),
        ("two dots", r#"{"n":1.2.3}"#.into()),
        ("bad escape", r#"{"s":"\x"}"#.into()),
        ("short unicode escape", r#"{"s":"\u12"}"#.into()),
        ("raw newline in string", "{\"s\":\"a\nb\"}".into()),
        ("raw NUL in string", "{\"s\":\"a\0b\"}".into()),
        ("raw NUL outside string", "{}\0".into()),
        ("unterminated string", r#"{"s":"abc}"#.into()),
        ("unterminated object", r#"{"a":1"#.into()),
        ("128 containers", too_deep),
    ]
}

pub fn accepted() -> Vec<(&'static str, String)> {
    let wide = format!(
        "{{{}}}",
        (0..10_000)
            .map(|i| format!("\"k{i}\":{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    vec![
        ("empty object", "{}".into()),
        (
            "big integer",
            r#"{"n":123456789012345678901234567890}"#.into(),
        ),
        ("huge exponent", r#"{"n":1e999999999999999999999}"#.into()),
        ("negative zero", r#"{"n":-0}"#.into()),
        ("fraction", r#"{"n":0.0,"m":1E+2,"k":-1.5e-3}"#.into()),
        ("unicode", r#"{"s":"é日本語 😀"}"#.into()),
        ("surrogate pair", r#"{"s":"\ud83d\ude00"}"#.into()),
        ("surrogate pair key", r#"{"\ud83d\ude00":1}"#.into()),
        (
            "escaped and literal keys that differ",
            r#"{"a":1,"\u0062":2}"#.into(),
        ),
        ("escaped NUL value", r#"{"s":"\u0000"}"#.into()),
        ("escaped NUL key", r#"{"\u0000":"x","a\u0000b":1}"#.into()),
        (
            "sibling repeated keys",
            r#"{"a":{"x":1},"b":{"x":2},"c":[{"x":3},{"x":4}]}"#.into(),
        ),
        ("empty key", r#"{"":1}"#.into()),
        (
            "parser token key",
            r#"{"$serde_json::private::Number":"1"}"#.into(),
        ),
        ("json whitespace", " \t\r\n{ \"a\" :\n1 }\r\n".into()),
        ("127 containers", nested(127)),
        ("wide object", wide),
    ]
}

pub fn equivalent() -> Vec<(&'static str, &'static str)> {
    vec![
        (r#"{"a":1,"b":2}"#, r#"{"b":2,"a":1}"#),
        (r#"{"a":1}"#, " { \"a\" :\t1 }\n"),
        (r#"{"a":1}"#, r#"{"\u0061":1}"#),
        (r#"{"a":"x"}"#, r#"{"a":"\u0078"}"#),
        (r#"{"s":"a/b"}"#, r#"{"s":"a\/b"}"#),
        (r#"{"s":"é"}"#, r#"{"s":"\u00e9"}"#),
        (r#"{"s":"\u00e9"}"#, r#"{"s":"\u00E9"}"#),
        (r#"{"s":"😀"}"#, r#"{"s":"\ud83d\ude00"}"#),
        (r#"{"s":"\ud83d\ude00"}"#, r#"{"s":"\uD83D\uDE00"}"#),
        (r#"{"é":1}"#, r#"{"\u00e9":1}"#),
        (r#"{"😀":1}"#, r#"{"\ud83d\ude00":1}"#),
        (
            r#"{"o":{"a":1,"b":[1,{"c":2,"d":3}]}}"#,
            r#"{"o":{"b":[1,{"d":3,"c":2}],"a":1}}"#,
        ),
        (r#"{"n":1.0}"#, r#"{ "n" : 1.0 }"#),
    ]
}

pub fn different() -> Vec<(&'static str, &'static str)> {
    vec![
        (r#"{"n":1}"#, r#"{"n":1.0}"#),
        (r#"{"n":1e2}"#, r#"{"n":1E2}"#),
        (r#"{"n":1e2}"#, r#"{"n":1e+2}"#),
        (r#"{"n":0}"#, r#"{"n":-0}"#),
        (r#"{"n":1.0}"#, r#"{"n":1.00}"#),
        (r#"{"n":100}"#, r#"{"n":1e2}"#),
        (
            r#"{"n":18446744073709551616}"#,
            r#"{"n":18446744073709551617}"#,
        ),
        (r#"{"n":1.0000000000000001}"#, r#"{"n":1.0}"#),
        (r#"{"a":[1,2]}"#, r#"{"a":[2,1]}"#),
        (r#"{"s":"é"}"#, "{\"s\":\"e\u{301}\"}"),
        (r#"{"s":"\u00e9"}"#, r#"{"s":"e\u0301"}"#),
        (r#"{"s":"é"}"#, r#"{"s":"\u00e8"}"#),
        (r#"{"s":"\ud83d\ude00"}"#, r#"{"s":"\ud83d\ude01"}"#),
        (r#"{"a":1}"#, r#"{"\u0062":1}"#),
        (r#"{"s":"a"}"#, r#"{"s":"A"}"#),
        (r#"{"v":true}"#, r#"{"v":"true"}"#),
        (r#"{"v":null}"#, r#"{"v":"null"}"#),
        (r#"{"a":1}"#, r#"{"a":1,"b":2}"#),
    ]
}

/// Guards the corpus itself: a case that is meant to exercise an escape must
/// still contain it, and a pair meant to show equivalence across encodings must
/// differ in bytes. Without this a self-comparison passes any parser.
pub fn assert_corpus_exercises_escapes() {
    for (name, payload) in rejected().into_iter().chain(accepted()) {
        let escape_case = name.contains("escape") || name.contains("surrogate pair");
        if escape_case {
            assert!(payload.contains('\\'), "{name}: lost its JSON escape");
        }
    }
    for (first, second) in equivalent() {
        assert_ne!(first, second, "equivalent pair is a self-comparison");
    }
    for (first, second) in different() {
        assert_ne!(first, second, "different pair is a self-comparison");
    }
    let escaped_pairs = equivalent()
        .into_iter()
        .filter(|(a, b)| a.contains("\\u") != b.contains("\\u"))
        .count();
    assert!(
        escaped_pairs >= 6,
        "only {escaped_pairs} literal-versus-escaped equivalence pairs"
    );
}
