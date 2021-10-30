use serde::Serialize;
use serde_bytes::ByteBuf;
use std::collections::BTreeMap;

fn s<T: ?Sized + Serialize>(v: &T) -> String {
    crate::to_string(v).unwrap()
}

fn b(bytes: &[u8]) -> ByteBuf {
    ByteBuf::from(bytes.to_vec())
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
