xml: String = "<library><book title=\"Dune\" author=\"Herbert\" /><book title=\"1984\" author=\"Orwell\" /></library>"

var first_attrs: Hash[String, String] = {"init" => "init"}
var second_attrs: Hash[String, String] = {"init" => "init"}

tree: Result[XmlNode, String] = Xml.parse(xml)
match tree do
Ok(root) do
  match root do
  Element(tag, _root_attrs, children) do
    puts tag
    first_child: XmlNode = children[0]
    second_child: XmlNode = children[1]
    match first_child do
    Element(_book1_tag, book1_attrs, _book1_children) do
      first_attrs = book1_attrs
    end
    Text(_book1_text) do
    end
    end
    match second_child do
    Element(_book2_tag, book2_attrs, _book2_children) do
      second_attrs = book2_attrs
    end
    Text(_book2_text) do
    end
    end
  end
  Text(_root_text) do
    puts "unexpected text root"
  end
  end
end
Err(msg) do
  puts msg
end
end

# `Hash[String, String]` has no `.get` (plan 25's own Decision log:
# Hash `get`/`set` reuse `Expr::Index`, not a method — a real, plan-
# text-contradicting finding, since plan 124's own Concrete Proof
# assumed a `.get`-returning-`Option[String]` method that does not
# exist), and `[]` indexing a `Hash[String, V]` by a `String` key is
# itself unimplemented in codegen (`build_hash_lookup` only supports
# `Int64` keys — confirmed by a real, previously-undisclosed codegen
# panic found by running exactly this). `.each` is the one real,
# already-working way to read a `Hash[String, String]`'s pairs (plan
# 121's own CSV demo already establishes `Pair[K, V]`'s `.key`/
# `.value`) — but ONLY as a bare top-level statement, never nested
# inside `if`/`while`/`match` (`call_named_proc`'s own doc comment:
# an inline `.each do |x| ... end` block is hoisted to a top-level
# compiled `Proc`, and the only argument shape codegen accepts at the
# call site is a plain top-level `Let`-bound name) — a second real,
# disclosed limitation found by running it, broader than plan 121's
# own already-disclosed "nested inside `Ok(v)`" version of the same
# gap. Both attribute maps are hoisted to top-level `var`s above and
# read here, after every enclosing `match` has already closed.
first_attrs.each do |pair: Pair[String, String]|
  if pair.key == "title" do
    puts pair.value
  end
end
second_attrs.each do |pair: Pair[String, String]|
  if pair.key == "title" do
    puts pair.value
  end
end

reader: XmlReader = Xml.reader_from_string(xml)
event: XmlEvent = reader.next_event
match event do
StartElement(tag, _attrs) do
  puts tag
end
EndElement(_tag) do
end
TextContent(_text) do
end
Eof do
end
end
