fn main() {
  let arr: [i64; 20] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
  ];
  let mut total: i64 = 0;
  let mut rep: i64 = 0;
  while rep < 1_000_000 {
    let mut i: usize = 0;
    while i < 20 {
      total += arr[i];
      i += 1;
    }
    rep += 1;
  }
  println!("{total}");
}
