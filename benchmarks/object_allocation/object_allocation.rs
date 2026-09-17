struct Box2 {
  value: i64,
}

fn main() {
  let mut total: i64 = 0;
  let mut i: i64 = 0;
  while i < 1_000_000 {
    let b = Box::new(Box2 { value: i });
    total += b.value;
    i += 1;
  }
  println!("{total}");
}
