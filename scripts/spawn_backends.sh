pushd tests/fake-backend
go build -o server .
popd
./tests/fake-backend/server --port 8090 &
SERVERONE_PID=$!
./tests/fake-backend/server --port 8091 &
SERVERTWO_PID=$!

sleep 1

echo "PID: ${SERVERONE_PID} ${SERVERTWO_PID}"


