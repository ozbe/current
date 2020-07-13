# current

## Run

```bash
# start redis
$ docker-compose up

# start server 
$ cargo run --package server

# start cli instance
$ cargo run --package cli
```

## Troubleshooting

### Follow chat

```bash
$ redis-cli
$ subscribe chat
...
```

### Clear user id count

Restart Redis or clear `user_id` count

```bash
$ redis-cli del user_id
```