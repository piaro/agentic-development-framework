class OrderService
  def place_order(order, redis, payload)
    order.save!
    redis.xadd("orders", payload)
  end
end
