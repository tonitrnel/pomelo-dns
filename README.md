# Pomelo DNS

Pomelo DNS 是一个使用 Rust 编写的 DNS 服务器，它接受来自本地客户端的 DNS 查询，并根据来源 IP 决定特定域名的返回，

该项目主要目的是使用 DNS 控制是否访问网站的 IPv6 地址

## Features

- 根据请求者的 IPAddr 返回特定的记录
- 根据请求者的 IPAddr 决定上游服务器
- 根据请求者的 IPAddr 决定是否返回 Ipv6 记录
- 根据请求域名决定是否返回 Ipv6 记录
- 根据 Ipv6 地址所属国家决定是否返回 Ipv6 记录
- 根据 Ipv6 的可 ping 性决定是否返回 Ipv6 记录
- 支持 DoT、DoH【未实现，因未支持 TCP 复用，暂无该需求】
- 支持DNS64【未实现】
- 缓存【未实现，暂无该需求】

## Known issues and todos

1. ~~使用 tracing 优化日志输出~~
2. 解决 在 Docker 容器内方法访问的问题
3. 解决 其他设备无法访问其它 DNS 服务器的奇怪问题
4. 实现 TCP 复用
5. 解决 Docker 容器内无法查询
6. 解决 子设备无法连接到其他 DNS 服务器
7. 实现 监测 `pomelo.conf` 文件并自动重启
8. 实现 支持直接指定 RR 类型，类似 `localhost   IN A    127.0.0.1`

## Installing

The project is still in development, so you'll need to build the Docker image yourself!

```bash
# build on client

make build
scp ./pomelo.img user@0.0.0.0:/...

# install on server
docker load -i pomelo.img
docker run -d \
        --name pomelo \
        --network host \
        --restart always \
        --cap-add NET_ADMIN \
        -v /etc/hosts:/etc/hosts:ro \
        -v /etc/localtime:/etc/localtime:ro \
        -v $(pwd)/pomelo.conf:/app/pomelo.conf:ro \
        pomelo:0.1.0
```

## License

- MIT license (LICENSE-MIT or http://opensource.org/licenses/MIT)

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as
defined in the MIT license, shall be licensed as above, without any additional terms or conditions.