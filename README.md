# serde_pyliteral

Serialize data to Python code that can be deserialized by [`ast.literal_eval`](https://docs.python.org/3/library/ast.html#ast.literal_eval) or this library.

This library could be interesting if you're looking for a format that:
- looks friendly for human eyes (not CBOR).
- supports non-utf8 binary data or non-string map keys (not JSON).

The library interface looks similar to `serde_json`'s, with two flavors of serialization: compact or pretty. The pretty output is inspired by [`pprint`](https://docs.python.org/3/library/pprint.html).

Example:

```rust
#[derive(serde::Serialize)]
struct Blob {
    name: &'static str,
    mtime: (f64, i32),
    readonly: bool,
    #[serde(with = "serde_bytes")]
    data: &'static [u8],
}

let blob = Blob {
    name: "名称\u{2029}",
    mtime: (1635745617.7, -25200),
    readonly: false,
    data: "数据".as_bytes(),
};

serde_pyliteral::to_writer_pretty(std::io::stdout(), &blob).unwrap();
```

Output:

```python
{"name": "名称\u2029",
 "mtime": (1635745617.7,
           -25200),
 "readonly": False,
 "data": b"\xe6\x95\xb0\xe6\x8d\xae"}
```

At present, this library focuses on simplicity and correctness (must round-trip, and must generate valid Python code). Performance hasn't been a priority.
