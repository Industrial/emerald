input: String = "name,role,notes\nAda,\"engineer, lead\",\"wrote \"\"the\nfirst\"\" program\"\nGrace,admiral,\n"

parsed: Result[Array[Array[String]], String] = Csv.parse(input)
match parsed do
Ok(rows) do
  header: Array[String] = rows[0]
  puts header[1]

  second_row: Array[String] = rows[1]
  puts second_row[1]
  puts second_row[2]
end
Err(msg) do
  puts msg
end
end

var first: Hash[String, String] = {"init" => "init"}
parsed_with_headers: Result[Array[Hash[String, String]], String] = Csv.parse_with_headers(input)
match parsed_with_headers do
Ok(records) do
  first = records[0]
end
Err(msg) do
  puts msg
end
end
first.each do |pair: Pair[String, String]|
  if pair.key == "role" do
    puts pair.value
  end
end
