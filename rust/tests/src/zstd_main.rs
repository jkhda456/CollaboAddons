fn main() {
    let text = b"hello hello hello hello hello hello hello hello";
    let packed = zstd::encode_all(&text[..], 3).unwrap();
    let back = zstd::decode_all(&packed[..]).unwrap();
    println!("zstd: {} -> {} -> {}, same: {}", text.len(), packed.len(), back.len(), back == text);
}
