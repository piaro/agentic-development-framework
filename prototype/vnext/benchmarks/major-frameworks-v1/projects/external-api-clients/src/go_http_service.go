package service
import "net/http"
func Notify(client *http.Client, request *http.Request) {
  client.Do(request)
}
