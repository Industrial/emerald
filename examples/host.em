require counter_actor

c: Counter = Counter.spawn(0)
c.register("counter1", 9000)
