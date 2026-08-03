import requests

def notify(payload):
    requests.post("https://example.test/orders", json=payload)
