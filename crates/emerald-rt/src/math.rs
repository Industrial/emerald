//! Plan 164 (Portable Math Functions) — a thin `Float64`-in-`Float64`-
//! out wrapping of `libm` (`rust-lang/libm`, the exact pure-Rust
//! fallback `core`'s own float math already uses on targets with no
//! OS-provided math library). Every function here is TOTAL over
//! `f64`'s full input range — out-of-domain input produces `NaN`/
//! `±Infinity`, the same convention libc's own `libm` uses, never a
//! panic — so none of these needs `Result`/`Option` on the Emerald
//! side, unlike almost every other `emerald-rt` module. `log` is
//! exposed as `.ln` at the Emerald surface (this module's own
//! function is still named `ln`, matching Rust's own `f64::ln`
//! naming) to avoid the real natural-log-vs-"the log function"
//! ambiguity plan 164's own Decision log names.

pub fn sin(x: f64) -> f64 {
  libm::sin(x)
}
pub fn cos(x: f64) -> f64 {
  libm::cos(x)
}
pub fn tan(x: f64) -> f64 {
  libm::tan(x)
}
pub fn asin(x: f64) -> f64 {
  libm::asin(x)
}
pub fn acos(x: f64) -> f64 {
  libm::acos(x)
}
pub fn atan(x: f64) -> f64 {
  libm::atan(x)
}
pub fn atan2(y: f64, x: f64) -> f64 {
  libm::atan2(y, x)
}
pub fn exp(x: f64) -> f64 {
  libm::exp(x)
}
pub fn exp2(x: f64) -> f64 {
  libm::exp2(x)
}
pub fn ln(x: f64) -> f64 {
  libm::log(x)
}
pub fn log2(x: f64) -> f64 {
  libm::log2(x)
}
pub fn log10(x: f64) -> f64 {
  libm::log10(x)
}
pub fn pow(x: f64, y: f64) -> f64 {
  libm::pow(x, y)
}
pub fn sqrt(x: f64) -> f64 {
  libm::sqrt(x)
}
pub fn cbrt(x: f64) -> f64 {
  libm::cbrt(x)
}
pub fn hypot(x: f64, y: f64) -> f64 {
  libm::hypot(x, y)
}
pub fn floor(x: f64) -> f64 {
  libm::floor(x)
}
pub fn ceil(x: f64) -> f64 {
  libm::ceil(x)
}
pub fn round(x: f64) -> f64 {
  libm::round(x)
}
pub fn trunc(x: f64) -> f64 {
  libm::trunc(x)
}

#[cfg(test)]
mod tests {
  use super::*;

  const EPSILON: f64 = 1e-10;

  fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < EPSILON
  }

  #[test]
  fn sqrt_two_matches_std_to_full_double_precision() {
    assert_eq!(sqrt(2.0), 2.0_f64.sqrt());
  }

  #[test]
  fn pow_two_to_the_ten_is_exact() {
    assert_eq!(pow(2.0, 10.0), 1024.0);
  }

  #[test]
  fn sqrt_of_negative_one_is_a_real_nan_not_a_panic() {
    assert!(sqrt(-1.0).is_nan());
  }

  #[test]
  fn sin_squared_plus_cos_squared_is_one_within_tolerance() {
    let s = sin(1.0);
    let c = cos(1.0);
    assert!(close(s * s + c * c, 1.0));
  }

  #[test]
  fn every_function_agrees_with_std_within_tolerance() {
    assert!(close(tan(0.5), 0.5_f64.tan()));
    assert!(close(asin(0.5), 0.5_f64.asin()));
    assert!(close(acos(0.5), 0.5_f64.acos()));
    assert!(close(atan(0.5), 0.5_f64.atan()));
    assert!(close(atan2(1.0, 2.0), 1.0_f64.atan2(2.0)));
    assert!(close(exp(1.0), 1.0_f64.exp()));
    assert!(close(exp2(3.0), 3.0_f64.exp2()));
    assert!(close(ln(std::f64::consts::E), 1.0));
    assert!(close(log2(8.0), 3.0));
    assert!(close(log10(1000.0), 3.0));
    assert!(close(cbrt(27.0), 3.0));
    assert!(close(hypot(3.0, 4.0), 5.0));
    assert!(close(floor(1.7), 1.0));
    assert!(close(ceil(1.2), 2.0));
    assert!(close(round(1.5), 2.0));
    assert!(close(trunc(1.9), 1.0));
  }
}
