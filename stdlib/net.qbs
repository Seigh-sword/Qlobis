/// HTTP access.

/// Perform a synchronous GET and return the response body.
defin get(url: str) -> str {
    return __net_get(url)
}

/// Perform a synchronous POST with `body` and return the response body.
defin post(url: str, body: str) -> str {
    return __net_post(url, body)
}

/// GET and return the body split into lines.
defin get_lines(url: str) -> list<str> {
    return __net_get(url).split("\n")
}
