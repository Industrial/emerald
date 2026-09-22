Http.serve(47303) do |req: HttpRequest|
  if req.path() == "/hello" do
    return HttpResponse.build(200, "hello #{req.method()}")
  end
  return HttpResponse.build(404, "not found")
end
