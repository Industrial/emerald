# Plan 127 (INI Configuration Files) — `rust-ini` wrapped behind an
# `IniDocument` resource handle (plan 93's own registry, the same
# shape `Regex`/`XmlReader` already use). `Ini.parse` returns a real
# `Result[IniDocument, IniError]` — plan 195's Typed Domain Errors
# convention, applied fresh here since this plan executes after plan
# 195 landed (this plan never shipped a `Result[T, String]` shape the
# way plan 118/122 originally did). `IniDocument#get`/`#key_count` are
# two more real instance methods on that handle; `.new`/`#set`/
# `#write` round-trip through `Ini.load` below, via a real `Tempfile`
# (plan 147) rather than a hardcoded path — `temp_files_proof.em`'s
# own established convention for a real file this proof can safely
# write to and clean up.
#
# This grammar has no `case`/`when`/`else` keywords (plan 71's own
# real, disclosed correction, already established by `json_demo.em`) —
# `match <scrutinee> do <Variant>(<binds>) do ... end ... end` is the
# real, current syntax.
doc: Result[IniDocument, IniError] = Ini.parse("[server]\nhost=localhost\nport=8080\n")

match doc do
Ok(d) do
  host: Option[String] = d.get("server", "host")
  match host do
  Some(h) do
    puts h
  end
  None do
    puts "no host"
  end
  end

  port: Option[String] = d.get("server", "port")
  match port do
  Some(p) do
    puts p
  end
  None do
    puts "no port"
  end
  end

  puts d.key_count("server")
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  Io(detail) do
    puts detail
  end
  Other(detail) do
    puts detail
  end
  end
end
end

# A second, real negative proof: a genuinely malformed document (an
# unterminated `[section` header, never finding a closing `]` before
# EOF) reaches the `Err` arm as a real, typed `IniError::Syntax` —
# `rust-ini`'s own real parser, not a simulated failure.
bad: String = "[unterminated\nkey=value\n"
bad_doc: Result[IniDocument, IniError] = Ini.parse(bad)
match bad_doc do
Ok(d) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  Io(detail) do
    puts detail
  end
  Other(detail) do
    puts detail
  end
  end
end
end

# A third proof: the write API. `IniDocument.new` builds a fresh,
# empty document; `#set` mutates it; `#write` serializes it to a real
# file (`Tempfile.create`'s own already-existing, empty file); `Ini.
# load` reads that same file back, `#get` confirming the round trip.
tf: Tempfile = Tempfile.create()
path: String = tf.path()

builder: IniDocument = IniDocument.new()
builder.set("app", "name", "demo")
write_result: Result[Void, IniError] = builder.write(path)
match write_result do
Ok(v) do
  reloaded: Result[IniDocument, IniError] = Ini.load(path)
  match reloaded do
  Ok(r) do
    name: Option[String] = r.get("app", "name")
    match name do
    Some(n) do
      puts n
    end
    None do
      puts "no name"
    end
    end
  end
  Err(e) do
    puts "reload failed"
  end
  end
end
Err(e) do
  puts "write failed"
end
end

tf.close()
