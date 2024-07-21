//! imcomplete object parser
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{error::Error, fmt::Display};
use swc_common::{input::StringInput, source_map::SmallPos, BytePos};
use swc_ecma_ast::{
    Decl, Expr, KeyValueProp, Lit, Prop, PropName, PropOrSpread, Script, Stmt, UnaryExpr, UnaryOp,
};
use swc_ecma_parser::{error::Error as SWCParseError, Parser};

#[derive(Debug)]
pub struct ParseError {
    kind: String,
    message: String,
}

impl From<SWCParseError> for ParseError {
    fn from(value: SWCParseError) -> Self {
        Self {
            kind: "parse error".into(),
            message: value.kind().msg().to_string(),
        }
    }
}

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl Error for ParseError {}

pub fn parse_source(source: &str) -> Result<Value, ParseError> {
    let source_file = StringInput::new(source, BytePos(0), BytePos::from_usize(source.len()));
    let mut parser = Parser::new(Default::default(), source_file, None);
    let script = parser.parse_script()?;

    parse_script(script).ok_or(ParseError {
        kind: "parse_script error".into(),
        message: "not find any value in script".into(),
    })
}

fn parse_script(script: Script) -> Option<Value> {
    let mut array = Vec::new();
    for stmt in script.body {
        if let Some(value) = parse_stmt(stmt) {
            array.push(value);
        }
    }

    match array.len() {
        0 => None,
        1 => array.pop(),
        _ => Some(Value::Array(array)),
    }
}

fn parse_stmt(stmt: Stmt) -> Option<Value> {
    match stmt {
        Stmt::Decl(decl) => {
            let Some(inits) = parse_decl(decl) else {
                return None;
            };
            let mut values = Vec::new();
            for init in inits {
                if let Some(value) = parse_expr(*init) {
                    values.push(value);
                }
            }

            match values.len() {
                0 => None,
                1 => Some(values.pop().unwrap()),
                _ => Some(Value::Array(values)),
            }
        }
        _ => None,
    }
}

#[inline]
fn parse_decl(decl: Decl) -> Option<Vec<Box<Expr>>> {
    match decl {
        Decl::Var(var) => Some(var.decls.into_iter().filter_map(|x| x.init).collect()),
        _ => None,
    }
}

fn parse_expr(expr: Expr) -> Option<Value> {
    match expr {
        Expr::Object(object) => {
            let props: Vec<KeyValueProp> = object
                .props
                .into_iter()
                .filter_map(|x| match x {
                    PropOrSpread::Prop(prop) => Some(*prop),
                    _ => None,
                })
                .filter_map(|x| match x {
                    Prop::KeyValue(kv) => Some(kv),
                    _ => None,
                })
                .collect();

            let mut map = serde_json::Map::new();

            for prop in props {
                if let Some(value) = parse_expr(*prop.value) {
                    let key = parse_prop_name(prop.key);
                    map.insert(key, value);
                }
            }

            Some(Value::Object(map))
        }
        Expr::Array(array_lit) => {
            let mut array = Vec::new();
            let elems = array_lit.elems.into_iter().filter_map(|x| x);
            for elem in elems {
                if let Some(value) = parse_expr(*elem.expr) {
                    array.push(value)
                }
            }
            Some(Value::Array(array))
        }
        Expr::Lit(lit) => parse_lit(lit),
        Expr::Unary(unary) => parse_unary(unary),
        // Expr::Bin(_) => None,
        // Expr::Ident(_) => None,
        // Expr::Fn(_) => None,
        // Expr::Arrow(_) => None,
        _ => None,
    }
}

#[inline]
fn parse_prop_name(key: PropName) -> String {
    match key {
        PropName::Str(str) => str.value.to_string(),
        PropName::Ident(ident) => ident.sym.to_string(),
        PropName::Num(num) => num.value.to_string(),
        _ => panic!("unknown key type"),
    }
}

#[inline]
fn parse_lit(lit: Lit) -> Option<Value> {
    match lit {
        Lit::Str(str) => Some(Value::String(str.value.to_string())),
        Lit::Num(num) => Some(Value::Number(
            serde_json::Number::from_f64(num.value).unwrap(),
        )),
        Lit::Bool(bool) => Some(Value::Bool(bool.value)),
        Lit::Null(_) => Some(Value::Null),
        _ => None,
    }
}

/// I don't want spend too much time on this, so this only can handle minus number
#[inline]
fn parse_unary(unary: UnaryExpr) -> Option<Value> {
    match unary.op {
        UnaryOp::Minus => {
            if let Some(Value::Number(number)) = parse_expr(*unary.arg) {
                let num = number.as_f64().unwrap();
                Some(Value::Number(serde_json::Number::from_f64(-num).unwrap()))
            } else {
                None
            }
        }
        UnaryOp::Plus => {
            if let ret @ Some(Value::Number(_)) = parse_expr(*unary.arg) {
                ret
            } else {
                None
            }
        }
        _ => None,
    }
}

pub trait CondKeys {
    fn keys<'a>() -> &'a [&'a str];
}

pub fn find_objects<T: CondKeys + DeserializeOwned>(value: Value) -> Vec<T> {
    let mut array = Vec::new();
    match value {
        Value::Object(map) => {
            let matched = T::keys().iter().all(|x| map.contains_key(*x));
            if matched {
                if let Ok(val) = serde_json::from_value(Value::Object(map.clone())) {
                    array.push(val);

                    return array;
                }
            }

            for (_, val) in map {
                array.extend(find_objects(val));
            }
        }
        Value::Array(elems) => {
            for elem in elems {
                array.extend(find_objects(elem));
            }
        }
        _ => {}
    }
    array
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::Value;

    use super::{find_objects, parse_source, CondKeys};

    const SOURCE: &str = r#"var data = {
    "object_key": {
        "string_key": "string",
        "bool_key": false,
        "number_key": 123456,
        "float_key": 3.1415926,
        "array_key": [1, +12, -24, 3.1415926, -0.3, true, false, null, "Hello World", {"object_in_array": true}]
    },
    "illegal stuff": [["down", "here"], "this is" + " killing me",...dont_collect_me],
    "chinese": "這可以處理中文嗎?", english: "can this handle same line?",
    3.1415926: "float(pi)",
    true: "bool",
    100: "number",
    null: "stop it",
    SOME_KEY: "key should be \"SOME_KEY\"",
    "don't parse function 1": function (name) {console.log(`Hello ${name}`)},
    "don't parse function 2": msg => console.log(msg),
    "end": true,
    }"#;

    const EXPECT: &str = r#"{
    "object_key": {
        "string_key": "string",
        "bool_key": false,
        "number_key": 123456.0,
        "float_key": 3.1415926,
        "array_key": [1.0, 12.0, -24.0, 3.1415926, -0.3, true, false, null, "Hello World", {"object_in_array": true}]
    },
    "illegal stuff": [["down", "here"]],
    "chinese": "這可以處理中文嗎?", "english": "can this handle same line?",
    "3.1415926": "float(pi)",
    "true": "bool",
    "100": "number",
    "null": "stop it",
    "SOME_KEY": "key should be \"SOME_KEY\"",
    "end": true
    }"#;

    #[test]
    fn test_parser() {
        let value = parse_source(SOURCE).unwrap();
        let expect: Value = serde_json::from_str(EXPECT).unwrap();
        assert_eq!(expect, value);
    }

    const SOURCE2: &str = r#"var data = {
        "try_this": {
            "string": "hello",
            "number": 12,
            "bool": true,
        }
    }"#;

    #[derive(Debug, PartialEq, Deserialize)]
    struct TryThis {
        string: String,
        number: f64,
        bool: bool,
    }

    impl CondKeys for TryThis {
        fn keys<'a>() -> &'a [&'a str] {
            return &["string", "number", "bool"];
        }
    }

    #[test]
    fn test_find_object() {
        let expect = TryThis {
            string: String::from("hello"),
            number: 12.0,
            bool: true,
        };

        let value = parse_source(SOURCE2).unwrap();
        let mut objects = find_objects::<TryThis>(value);
        let object = objects.pop().unwrap();
        assert_eq!(expect, object);
    }
}
