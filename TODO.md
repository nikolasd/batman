# BATMAN TODO

## Feature Requests

### Org Config: URL or File Path Support

**Status:** Not Started  
**Priority:** Medium  
**Labels:** enhancement, configuration

**Description:**
Currently, org config is loaded only from file paths. This should be enhanced to support either:
- A file path (current behavior)
- A URL (HTTP/HTTPS) for remote configuration

**Implementation Notes:**
- Modify `crates/runtime/src/config/merge.rs` `load_layer` function
- Detect if the path is a URL (starts with `http://` or `https://`)
- If URL, fetch the content and parse as YAML
- If file path, load from disk (current behavior)
- Add appropriate error handling for network failures
- Consider caching fetched URLs to avoid repeated network calls

**Example Usage:**
```bash
# File path (current)
batman serve --org-config /etc/batman/org.yaml

# URL (new)
batman serve --org-config https://config.example.com/org.yaml
```

**Dependencies:**
- Network access for URL fetching
- TLS certificate validation for HTTPS URLs
- Timeout handling for network requests

---

## Other Potential Features

- [ ] Add support for config templates
- [ ] Add config validation against schema before loading
- [ ] Add config versioning and migration support
- [ ] Add config encryption for sensitive values
