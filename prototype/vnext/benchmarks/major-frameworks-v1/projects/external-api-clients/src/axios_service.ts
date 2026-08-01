import axios from "axios";
export function notify(payload: unknown) {
  axios.post("https://example.test/orders", payload);
}
