use axum::http::{HeaderMap, header};

/// Select a language per request. Unsupported languages fall back to Simplified Chinese.
pub fn wants_english(headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let mut preferred = None;
    for item in value.split(',').take(20) {
        let mut parts = item.trim().split(';');
        let language = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        let english = match language.split('-').next().unwrap_or_default() {
            "en" => true,
            "zh" => false,
            _ => continue,
        };
        let weight = parts
            .find_map(|parameter| parameter.trim().strip_prefix("q="))
            .map_or(Some(1.0), |q| q.parse::<f32>().ok());
        let Some(weight) = weight.filter(|q| q.is_finite() && *q > 0.0 && *q <= 1.0) else {
            continue;
        };
        if preferred.is_none_or(|(best, _)| weight > best) {
            preferred = Some((weight, english));
        }
    }
    preferred.is_some_and(|(_, english)| english)
}

// Codes describe conditions, not the current language. Existing clients can continue reading `error`.
const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "素材或目录不存在",
        "The asset or library does not exist.",
        "not_found",
    ),
    (
        "本地服务仅接受 localhost 或回环地址访问",
        "This local service only accepts localhost or loopback addresses.",
        "local_host_required",
    ),
    (
        "拒绝跨站点请求",
        "Cross-origin requests are not allowed.",
        "cross_origin_denied",
    ),
    (
        "需要用户名 picsoc 和设置的密码",
        "Sign in with username picsoc and your configured password.",
        "authentication_required",
    ),
    (
        "请求方法无效",
        "This request method is not supported.",
        "method_not_allowed",
    ),
    (
        "请求内容过大",
        "The request body is too large.",
        "payload_too_large",
    ),
    (
        "接口不存在",
        "This API endpoint does not exist.",
        "endpoint_not_found",
    ),
    (
        "请求参数无效",
        "The request parameters are invalid.",
        "invalid_request",
    ),
    (
        "目录路径过长",
        "The directory path is too long.",
        "path_too_long",
    ),
    (
        "请选择绝对目录路径",
        "Please select an absolute directory path.",
        "absolute_path_required",
    ),
    (
        "请填写此服务所在机器的绝对目录路径",
        "Enter an absolute directory path on the machine running this service.",
        "absolute_path_required",
    ),
    (
        "路径必须是文件夹",
        "The path must point to a directory.",
        "directory_required",
    ),
    (
        "目录路径必须能转换为 Unicode",
        "The directory path must be valid Unicode.",
        "invalid_unicode_path",
    ),
    (
        "目录名称需要 1–100 个字符",
        "The library name must contain 1–100 characters.",
        "invalid_library_name",
    ),
    (
        "不能把 Picsoc 数据目录作为素材目录",
        "The Picsoc data directory cannot be used as an image library.",
        "data_directory_forbidden",
    ),
    (
        "该目录已经添加",
        "This directory has already been added.",
        "library_already_exists",
    ),
    (
        "搜索内容过长",
        "The search query is too long.",
        "query_too_long",
    ),
    ("排序方式无效", "The sort order is invalid.", "invalid_sort"),
    (
        "图片格式无效",
        "The image format is invalid.",
        "invalid_format",
    ),
    (
        "每张图片最多 50 个标签，每个标签最多 50 个字符",
        "Each image can have up to 50 tags, with up to 50 characters per tag.",
        "invalid_tags",
    ),
    (
        "缩略图服务暂不可用",
        "The thumbnail service is temporarily unavailable.",
        "thumbnail_service_unavailable",
    ),
    (
        "请求的字节范围无效",
        "The requested byte range is invalid.",
        "invalid_range",
    ),
    (
        "前端尚未构建",
        "The web interface has not been built yet.",
        "frontend_unavailable",
    ),
    ("文件不存在", "The file does not exist.", "file_not_found"),
    (
        "素材路径无效",
        "The asset path is invalid.",
        "invalid_asset_path",
    ),
    (
        "素材目录不可访问",
        "The library directory is unavailable.",
        "library_unavailable",
    ),
    (
        "素材目录已改变，请重新添加目录",
        "The library directory has changed. Remove and add the directory again.",
        "library_directory_changed",
    ),
    (
        "素材目录已改变，请重新添加",
        "The library directory has changed. Remove and add the directory again.",
        "library_directory_changed",
    ),
    (
        "原图不存在或不可访问",
        "The original image is missing or unavailable.",
        "original_unavailable",
    ),
    (
        "素材路径超出了素材目录",
        "The asset path is outside its library directory.",
        "asset_path_outside_library",
    ),
    (
        "图片超过 256 MiB，已跳过缩略图",
        "The image exceeds 256 MiB. Thumbnail generation was skipped.",
        "image_file_too_large",
    ),
    (
        "图片解码超过 128 MiB，已跳过缩略图",
        "The decoded image exceeds 128 MiB. Thumbnail generation was skipped.",
        "image_decode_too_large",
    ),
    (
        "缩略图缓存路径错误",
        "The thumbnail cache path is invalid.",
        "thumbnail_cache_path_invalid",
    ),
    (
        "无法打开数据库",
        "The database could not be opened.",
        "database_unavailable",
    ),
    (
        "数据库锁异常",
        "The database lock is unavailable.",
        "database_lock_unavailable",
    ),
    (
        "文件名无法转换为 Unicode",
        "The file name is not valid Unicode.",
        "invalid_unicode_filename",
    ),
    (
        "缩略图队列已停止",
        "The thumbnail queue has stopped.",
        "thumbnail_queue_stopped",
    ),
];

const PREFIXES: &[(&str, &str, &str)] = &[
    (
        "目录不可访问：",
        "The directory is unavailable: ",
        "directory_unavailable",
    ),
    (
        "目录不存在或无权限：",
        "The directory does not exist or permission was denied: ",
        "directory_unavailable",
    ),
    (
        "目录不可读取：",
        "The directory cannot be read: ",
        "directory_unreadable",
    ),
    (
        "扫描未完成，保留原有索引：",
        "The scan could not finish. Existing index entries were retained: ",
        "scan_incomplete",
    ),
];

pub fn error_message(message: &str, english: bool) -> (String, &'static str) {
    if let Some((_, translation, code)) = MESSAGES
        .iter()
        .find(|(original, _, _)| *original == message)
    {
        return (
            if english {
                (*translation).to_owned()
            } else {
                message.to_owned()
            },
            code,
        );
    }
    for (prefix, translation, code) in PREFIXES {
        if let Some(detail) = message.strip_prefix(prefix) {
            let detail = error_message(detail, english).0;
            return (
                if english {
                    format!("{translation}{detail}")
                } else {
                    message.to_owned()
                },
                code,
            );
        }
    }
    (message.to_owned(), "request_failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_negotiation_respects_quality_order_and_fallback() {
        for (language, english) in [
            ("en", true),
            ("en-US,zh-CN;q=0.8", true),
            ("zh-CN,en;q=0.8", false),
            ("en;q=0.1,zh-CN;q=0.9", false),
            ("de,en;q=0.5", true),
            ("fr", false),
            ("en;q=0,zh;q=1", false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_LANGUAGE, language.parse().unwrap());
            assert_eq!(wants_english(&headers), english, "{language}");
        }
        assert!(!wants_english(&HeaderMap::new()));
    }
    #[test]
    fn dynamic_errors_keep_system_details_and_stable_codes() {
        let message = "目录不可访问：Permission denied (os error 13)";
        let (english, code) = error_message(message, true);
        assert_eq!(
            english,
            "The directory is unavailable: Permission denied (os error 13)"
        );
        assert_eq!(code, "directory_unavailable");
        assert_eq!(error_message(message, false).0, message);
        assert_eq!(
            error_message("Permission denied (os error 13)", true).0,
            "Permission denied (os error 13)"
        );
    }
}
