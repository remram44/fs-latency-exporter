Filesystem Latency Exporter
===========================

This is an exporter for Prometheus, measuring the latency of random read to a local file.

It can be used with networked file systems as well, if they are mounted on the local machine.

Reads use direct I/O when available (UNIX), picking a random 4096-byte block in the file.

Example usage:

```
# Create a large file on the target filesystem
dd if=/dev/urandom of=/mnt/100GB.bin bs=10M count=10000 status=progress

# Run the exporter
./fs-latency-exporter --metrics 127.0.0.1:8080 /mnt/100GB.bin

# Grab the metrics (in another terminal, or via Prometheus)
curl -s http://127.0.0.1:8080/metrics
```

The exposed metrics are:

- `errors_total`, a counter of errors encountered when reading and seeking
- `read_time_seconds`, a histogram for the duration of the random reads
