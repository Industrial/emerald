unsafe extern "C" {
  fn llabs(x: Int64): Int64
  fn strlen(s: String): Int64
  fn strstr(haystack: String, needle: String): CString
}

x: Int64 = llabs(-42)
puts x

n: Int64 = strlen("hello")
puts n

found: Option[String] = String.from_cstring(strstr("hello world", "world"))
puts found ?? "not found"

missing: Option[String] = String.from_cstring(strstr("hello world", "xyz"))
puts missing ?? "not found"
