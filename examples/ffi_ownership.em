# Plan 85's `ownership-actor-ffi-integration` (`spec/OWNERSHIP.md` §7):
# `own`/`borrow` typing at an `unsafe extern "C"` boundary — "no new
# mechanism, the FFI boundary is just another own/borrow-typed call
# site." Legal ONLY on an extern fn's own RETURN type in v1 (extern
# PARAMETERS stay exactly as strict/unannotated as before this plan).
#
# `strdup` (real glibc) allocates a FRESH heap copy — the caller (this
# program) now owns that memory, exactly what `own CString` documents.
# `strstr` (also real glibc, and already `examples/c_ffi.em`'s own
# worked example) returns a pointer INTO its `haystack` argument's own
# storage, never a fresh allocation — exactly what `borrow CString`
# documents. Both annotations compile to the IDENTICAL C ABI call
# either way (a real, disclosed, deliberate non-effect on codegen —
# see `emerald-codegen`'s own `strip_ownership_annotations_in_item`
# doc comment for the `Item::Extern` case this plan added).
# Wrapped in a real function body, not left as top-level statements —
# plan 56/83's own pre-existing, disclosed scope boundary means the
# message-safety/ownership liveness passes below only ever run inside a
# function/method body, never over bare top-level statements (this is
# not something plan 85 introduces or is asked to close; stating it
# here so this example genuinely exercises the real check, rather than
# only appearing to).
unsafe extern "C" {
  fn strdup(s: String): own CString
  fn strstr(haystack: String, needle: String): borrow CString
}

fn run: Void do
  copy: Option[String] = String.from_cstring(strdup("owned copy"))
  puts copy ?? "strdup failed"

  # A `borrow`-returning call's result may be used directly, inline, in
  # the same expression — never bound to a name (see
  # `ffi_borrow_return_reject.em`'s companion file for the real,
  # specific rejection this would otherwise produce).
  found: Option[String] = String.from_cstring(strstr("hello world", "world"))
  puts found ?? "not found"
end

run()
