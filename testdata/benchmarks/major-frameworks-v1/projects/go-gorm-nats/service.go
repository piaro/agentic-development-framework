package orders

func PlaceOrder(db *gorm.DB, nc *nats.Conn, order *Order) {
	db.Create(order)
	nc.Publish("orders", []byte("created"))
}
