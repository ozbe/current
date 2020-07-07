# current

## Run

```bash
# start redis
$ docker-compose up

# start server 
$ cargo run
```

## Troubleshooting

### Follow chat
```bash
$ redis-cli
$ subscribe chat
...
```