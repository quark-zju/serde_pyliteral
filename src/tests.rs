use serde::de;
use serde::Serialize;
use serde_bytes::ByteBuf;
use serde_json::Value;
use std::collections::BTreeMap;

fn s<T: ?Sized + Serialize>(v: &T) -> String {
    crate::to_string(v).unwrap()
}

fn p<T: ?Sized + Serialize>(v: &T) -> String {
    let mut s = crate::to_string_pretty(v).unwrap();
    if s.contains('\n') {
        s = format!("\n{}", s);
    }
    s
}

fn b(bytes: &[u8]) -> ByteBuf {
    ByteBuf::from(bytes.to_vec())
}

fn d<T: de::DeserializeOwned>(s: &str) -> T {
    crate::from_str(s).unwrap()
}

#[test]
fn test_serialize_basic_types() {
    assert_eq!(s(&42), "42");
    assert_eq!(s(&'"'), "'\"'");
    assert_eq!(s(&"汉字abc\u{f234}"), r#""汉字abc\uf234""#);

    assert_eq!(s(&b(b"123\0\n\xff\0")), r#"b"123\0\n\xff\0""#);

    assert_eq!(s(&[1, 2, 3]), "(1,2,3)");
    assert_eq!(s(&[1, 2, 3][..]), "[1,2,3]");
    assert_eq!(s(&[Some(true), Some(false), None]), "(True,False,None)");

    assert_eq!(s(&()), "()");
    assert_eq!(s(&(true, false, "x")), "(True,False,\"x\")");

    assert_eq!(s(&vec!["a", "bc"]), "[\"a\",\"bc\"]");
}

#[test]
fn test_serialize_map() {
    let mut m = BTreeMap::new();
    assert_eq!(s(&m), "{}");
    m.insert(1, "a");
    m.insert(2, "b");
    assert_eq!(s(&m), r#"{1:"a",2:"b"}"#);
}

#[test]
fn test_serialize_struct() {
    #[derive(Serialize)]
    struct A {
        a: i32,
        b: bool,
        c: &'static str,
        d: ByteBuf,
        e: (u8, u8),
        f: Option<B>,
        g: C,
        h: Vec<Option<D>>,
        i: E,
    }
    #[derive(Serialize)]
    struct B(i32);
    #[derive(Serialize)]
    struct C(char, Option<bool>);
    #[derive(Serialize)]
    struct D;
    #[derive(Serialize)]
    struct E {
        inner: u32,
    }

    let a = A {
        a: -10,
        b: false,
        c: "abc",
        d: b(b"123"),
        e: (2, 5),
        f: Some(B(0)),
        g: C(' ', None),
        h: vec![Some(D), None],
        i: E { inner: 1 },
    };
    assert_eq!(
        s(&a),
        "{\"a\":-10,\"b\":False,\"c\":\"abc\",\"d\":b\"123\",\"e\":(2,5),\"f\":0,\"g\":(\" \",None),\"h\":[(),None],\"i\":{\"inner\":1}}"
    );
}

#[test]
fn test_serialize_enum() {
    #[derive(Serialize)]
    struct D;

    #[derive(Serialize)]
    enum A {
        A,
        B(u32),
        C(u32, u32),
        D(D),
        E { a: u32, b: u32 },
    }
    assert_eq!(s(&A::A), "{\"A\":()}");
    assert_eq!(s(&A::B(1)), "{\"B\":1}");
    assert_eq!(s(&A::C(1, 2)), "{\"C\":(1,2)}");
    assert_eq!(s(&A::D(D)), "{\"D\":()}");
    assert_eq!(s(&A::E { a: 1, b: 2 }), "{\"E\":{\"a\":1,\"b\":2}}");
}

#[test]
fn test_pretty() {
    assert_eq!(p(&[1]), "(1,)");
    assert_eq!(p(&[1, 2]), "\n(1,\n 2)");

    assert_eq!(p(&vec![1]), "[1]");
    assert_eq!(p(&vec![1, 2]), "\n[1,\n 2]");
    assert_eq!(p(&vec![vec![1], vec![2, 2]]), "\n[[1],\n [2,\n  2]]");

    let mut m = BTreeMap::new();
    assert_eq!(p(&m), "{}");
    m.insert(1, "a");
    assert_eq!(p(&m), "{1: \"a\"}");
    m.insert(222, "b");
    assert_eq!(p(&m), "\n{1: \"a\",\n 222: \"b\"}");

    let mut m = BTreeMap::new();
    m.insert((1, (2, 4)), vec![vec![1], vec![2]]);
    m.insert((222, (333, 0)), vec![vec![3, 4], vec![5]]);
    assert_eq!(
        p(&m),
        r#"
{(1,(2,4)): [[1],
             [2]],
 (222,(333,0)): [[3,
                  4],
                 [5]]}"#
    );

    #[derive(Serialize)]
    struct A {
        foo: Vec<u32>,
        inner: Vec<A>,
    }
    let a = A {
        foo: vec![],
        inner: vec![A {
            foo: vec![3],
            inner: vec![A {
                foo: vec![5, 6],
                inner: vec![],
            }],
        }],
    };
    assert_eq!(
        p(&a),
        r#"
{"foo": [],
 "inner": [{"foo": [3],
            "inner": [{"foo": [5,
                               6],
                       "inner": []}]}]}"#
    );
}

#[test]
fn test_deserialize_basic() {
    let v: String = d(r#"'abcd文字\0\n\t\\\uf230"'"#);
    assert_eq!(v, "abcd文字\u{0}\n\t\\\u{f230}\"");

    let v: ByteBuf = d(r#"b"\0\n\t\x12\xff123 \\\'\"\r""#);
    assert_eq!(v, [0, 10, 9, 18, 255, 49, 50, 51, 32, 92, 39, 34, 13]);

    let v: u64 = d("18446744073709551613");
    assert_eq!(v, 18446744073709551613);

    let v: i64 = d("-9223372036854775801");
    assert_eq!(v, -9223372036854775801);

    let v: Option<bool> = d("True");
    assert_eq!(v, Some(true));
    let v: Option<bool> = d("False");
    assert_eq!(v, Some(false));
    let v: Option<bool> = d("None");
    assert_eq!(v, None);

    let v: char = d("'写'");
    assert_eq!(v, '写');

    let v: () = d(" ()");
    assert_eq!(v, ());
}

#[test]
fn test_deserialize_any() {
    let v: Value = d(r#"
        # Comments are skipped.
        # Spaces are skipped too.
        123"#);
    assert_eq!(v.to_string(), "123");

    let v: Value = d("'abc'");
    assert_eq!(v.to_string(), "\"abc\"");

    let v: Value = d("True");
    assert_eq!(v.to_string(), "true");

    let v: Value = d("None");
    assert_eq!(v.to_string(), "null");

    let v: Value = d("[1, True, 'abc', None]");
    assert_eq!(v.to_string(), "[1,true,\"abc\",null]");

    let v: Value = d("(1, (2, 3), (4,),)");
    assert_eq!(v.to_string(), "[1,[2,3],[4]]");
}

#[test]
fn test_deserialize_list() {
    let v: [bool; 3] = d(" ( True,False, True ) ");
    assert_eq!(v, [true, false, true]);

    let v: Vec<String> = d(r#"['a',"","b"," ",]"#);
    assert_eq!(v, ["a", "", "b", " "]);

    let v: Vec<Vec<u8>> = d(r#"[[3,4,],[5],[]]"#);
    assert_eq!(v, [vec![3, 4], vec![5], vec![]]);
}
