def place_order(order, statement, session, tasks):
    session.add(order)
    session.execute(statement)
    tasks.process_order.delay(order)
