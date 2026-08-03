<?php
function placeOrder($order, $topic, $payload) {
    $order->save();
    $topic->publish($payload);
}
