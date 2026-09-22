input: String = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[package.tags]\nlang = \"emerald\"\n"

parsed: Result[JsonValue, String] = Toml.parse(input)
match parsed do
Ok(doc) do
  pkg: Option[JsonValue] = doc.get("package")
  match pkg do
  Some(pkg_val) do
    name: Option[JsonValue] = pkg_val.get("name")
    match name do
    Some(name_val) do
      match name_val do
      JsonString(s) do
        puts s
      end
      _ do
        puts "missing"
      end
      end
    end
    None do
      puts "missing"
    end
    end
  end
  None do
    puts "missing"
  end
  end
end
Err(msg) do
  puts msg
end
end

bad: String = "not = valid = toml"
bad_parsed: Result[JsonValue, String] = Toml.parse(bad)
match bad_parsed do
Ok(doc) do
  puts "unexpected ok"
end
Err(msg) do
  puts msg
end
end
