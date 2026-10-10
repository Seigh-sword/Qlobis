/// Filesystem access.
///
/// Paths are plain strings; errors raise through the target runtime and can
/// be caught with `try { ... } throw err { ... }`.

/// Read a whole file as a string.
defin read(path: str) -> str {
    return __fs_read(path)
}

/// Read a file and split it into lines.
defin read_lines(path: str) -> list<str> {
    let text = __fs_read(path)
    let lines = text.split("\n")
    if lines.len() > 0 and lines.get(lines.len() - 1) == "" {
        lines.remove(lines.len() - 1)
    }
    return lines
}

/// Write `data` to `path`, replacing previous contents.
defin write(path: str, data: str) -> bool {
    return __fs_write(path, data)
}

/// Write `data` to `path` and end with a newline.
defin write_line(path: str, data: str) -> bool {
    return __fs_write(path, data + "\n")
}

/// Append `data` to `path`, creating it when missing.
defin append(path: str, data: str) -> bool {
    let prev = ""
    if __fs_exists(path) {
        prev = __fs_read(path)
    }
    return __fs_write(path, prev + data)
}

/// Write a list of lines to `path`.
defin write_lines(path: str, lines: list<str>) -> bool {
    let out = ""
    let i = 0
    while i < lines.len() {
        out = out + lines.get(i)
        if i + 1 < lines.len() {
            out = out + "\n"
        }
        i = i + 1
    }
    return __fs_write(path, out)
}

/// List file names in a directory (sorted).
defin list(path: str) -> list<str> {
    let names = __fs_list(path)
    names.sort()
    return names
}

/// Does `path` exist?
defin exists(path: str) -> bool {
    return __fs_exists(path)
}

/// Delete a file. Returns true when it existed.
defin remove(path: str) -> bool {
    return __fs_remove(path)
}

/// Create a directory (and parents). Returns true on success.
defin mkdir(path: str) -> bool {
    return __fs_mkdir(path)
}
