fn square(n: i64) -> i64 {
  n * n
}

fn main() {
  let mut total: i64 = 0;
  let mut i: i64 = 0;
  while i < 1_000_000 {
    total += square(i);
    i += 1;
  }
  println!("{total}");
}
