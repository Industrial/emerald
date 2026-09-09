require counter_actor

handle: Counter = Counter.remote("127.0.0.1:9000", "counter1")
handle.increment
handle.increment
handle.increment
handle.report
