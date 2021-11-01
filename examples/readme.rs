fn main() {
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
}
