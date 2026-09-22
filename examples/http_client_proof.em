first: Result[HttpResponse, String] = Http.get("https://httpbin.org/status/200")
match first do
Ok(r) do
  puts r.status()
end
Err(e) do
  puts e
end
end

not_found: Result[HttpResponse, String] = Http.get("https://httpbin.org/status/404")
match not_found do
Ok(r) do
  puts r.status()
end
Err(e) do
  puts e
end
end

fn fetch_status(url: String): Result[Int64, String] do
  r: HttpResponse = Http.get(url)?
  return Ok(r.status())
end

third: Result[Int64, String] = fetch_status("https://httpbin.org/status/500")
match third do
Ok(s) do
  puts s
end
Err(e) do
  puts e
end
end
