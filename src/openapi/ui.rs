/// Returns HTML that renders Scalar API reference UI, loading the
/// OpenAPI spec from `/openapi.json` on the same host.
pub(crate) fn scalar_html() -> String {
    r#"<!DOCTYPE html>
<html>
<head>
  <title>API Reference</title>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
</head>
<body>
  <script id="api-reference" data-url="/openapi.json"></script>
  <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
</body>
</html>"#
        .to_owned()
}
