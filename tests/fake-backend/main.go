package main

import (
	"flag"
	"fmt"
	"log"
	"net"

	"github.com/google/uuid"
)

var port string

func main() {
	flag.StringVar(&port, "port", "8090", "port to set for the fake TCP backend")
	flag.Parse()

	var id = uuid.New().String()

	addr := fmt.Sprintf("localhost:%s", port)
	listener, _ := net.Listen("tcp", addr)
	log.Println("Listening on", listener.Addr())

	for {
		conn, err := listener.Accept()
		if err != nil {
			log.Fatal(err)
		}
		defer conn.Close()
		fmt.Println("Accepted conn from", conn.RemoteAddr())

		go func(c net.Conn) {
			msg := fmt.Sprintf("Hello from backend server %s", id)
			c.Write([]byte(msg))
			c.Close()
		}(conn)
	}
}
