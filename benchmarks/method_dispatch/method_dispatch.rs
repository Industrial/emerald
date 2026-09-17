struct Adder {
  base: i64,
}

impl Adder {
  fn add(&self, n: i64) -> i64 {
    self.base + n
  }
}

fn main() {
  let a = Adder { base: 1 };
  let mut total: i64 = 0;
  let mut i: i64 = 0;
  while i < 10_000_000 {
    total += a.add(i);
    i += 1;
  }
  println!("{total}");
}
