# `derive Serializable` — synthesizes `to_json_value(self): JsonValue`
# from a class's own field list (plan 118's `JsonValue` enum, this
# session's `derive-serializable` leaf), the same synthesis-from-field-
# list mechanism `derive Comparable` (plan 61) already established for
# `==`.
#
# Real, disclosed scope limits, found while building this: (1) only
# four field types are supported directly — `Int64`, `Float64`,
# `String`, `Boolean` — a field of any other type (a nested class,
# `Array[T]`, `Option[T]`, ...) is a real `derive`-time compile error
# naming the field, not a silent skip. (2) an `Int64` field needs a
# brand-new `.to_f` conversion (there was previously no way at all to
# turn an `Int64` into a `Float64` in this language — `JsonNumber` only
# ever carries a `Float64`); this leaf adds `Int64#to_f`/`Float64#to_i`
# as a small prerequisite. (3) there is NO `derive`-synthesized
# `from_json_value` — this compiler has no class-level static-method
# dispatch at all (`ClassName.new`/`.spawn`/`.remote`/`.locate` are the
# only reserved bare-class-name call forms; a real user class has no
# way to be constructed without an instance already in hand), so the
# deserialization direction is deferred pending that prerequisite,
# not silently attempted.

class Person derive Serializable
  name: String
  age: Int64
  gpa: Float64
  active: Boolean

  fn initialize(name: String, age: Int64, gpa: Float64, active: Boolean): Void do
    @name = name
    @age = age
    @gpa = gpa
    @active = active
  end
end

p: Person = Person.new("Ada", 36, 3.9, true)
doc: JsonValue = p.to_json_value
puts doc.to_s
