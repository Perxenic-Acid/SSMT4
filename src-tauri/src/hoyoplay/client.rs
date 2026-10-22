use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Read, time::Duration};

pub const AUDIO: &[&str] = &["zh-cn", "en-us", "ja-jp", "ko-kr"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub game_id: String,
    pub biz: String,
    pub exe: String,
    pub channel: u32,
    pub sub_channel: u32,
    #[serde(skip)]
    pub launcher: &'static str,
}

pub fn servers(preset: &str) -> Vec<Server> {
    let ids = match preset {
        "GIMI" => ["1Z8W5NHUQb", "T2S0Gz4Dr2", "gopR6Cufr3", "hk4e"],
        "SRMI" => ["64kMb5iAWu", "EdtUqXfCHh", "4ziysqXOQ8", "hkrpg"],
        "ZZMI" | "ZZMIDX12" => ["x6znKlJ0xK", "HXAFlmYa17", "U5hbdsT9W7", "nap"],
        _ => return vec![],
    };
    ["cn", "bilibili", "global"]
        .iter()
        .enumerate()
        .map(|(index, id)| Server {
            id: id.to_string(),
            name: ["国服", "B服", "国际服"][index].into(),
            game_id: ids[index].into(),
            biz: format!("{}_{}", ids[3], if index == 2 { "global" } else { "cn" }),
            exe: match (preset, index) {
                ("GIMI", 2) => "GenshinImpact.exe",
                ("GIMI", _) => "YuanShen.exe",
                ("SRMI", _) => "StarRail.exe",
                _ => "ZenlessZoneZero.exe",
            }
            .into(),
            channel: if index == 1 { 14 } else { 1 },
            sub_channel: if index == 1 { 0 } else { 1 },
            launcher: match (index, preset) {
                (2, _) => "VYTpXlbWo8",
                (1, "GIMI") => "umfgRO5gh5",
                (1, "SRMI") => "6P5gHMNyK3",
                (1, _) => "xV0f4r1GT0",
                _ => "jGHBHlcOq1",
            },
        })
        .collect()
}

impl Server {
    pub fn api(&self) -> &'static str {
        if self.id == "global" {
            "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/"
        } else {
            "https://hyp-api.mihoyo.com/hyp/hyp-connect/api/"
        }
    }
    fn downloader(&self) -> &'static str {
        if self.id == "global" {
            "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild"
        } else {
            "https://downloader-api.mihoyo.com/downloader/sophon_chunk/api/getBuild"
        }
    }
}

pub fn http() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || validate_url(attempt.url().as_str()).is_err() {
                attempt.error("官方资源重定向不被允许")
            } else {
                attempt.follow()
            }
        }))
        .user_agent("SSMT/HoYoPlay")
        .build()
        .map_err(|e| e.to_string())
}

pub fn validate_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("资源网址无效：{e}"))?;
    let host = url.host_str().unwrap_or_default();
    let allowed = [
        "mihoyo.com",
        "hoyoverse.com",
        "yuanshen.com",
        "hoyocdn.com",
        "hoyoverse.cn",
        "bhsr.com",
        "starrails.com",
        "juequling.com",
        "zenlesszonezero.com",
    ]
    .iter()
    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")));
    if url.scheme() != "https"
        || !allowed
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        return Err(format!("不是支持的官方 HTTPS 资源域名：{host}"));
    }
    Ok(url)
}

pub fn get_bytes(client: &Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    validate_url(url)?;
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("官方资源获取失败：{e}"))?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err("官方响应超出大小限制".into());
    }
    let mut bytes = vec![];
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("官方响应超出大小限制".into());
    }
    Ok(bytes)
}

fn json(client: &Client, url: reqwest::Url) -> Result<Value, String> {
    let bytes = get_bytes(client, url.as_str(), 8 * 1024 * 1024)?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("官方接口响应无法解析：{e}"))?;
    if value["retcode"].as_i64() != Some(0) {
        return Err(format!(
            "官方接口拒绝请求：{} {}",
            value["retcode"], value["message"]
        ));
    }
    value
        .get("data")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| "官方接口未返回数据".into())
}

pub fn endpoint(server: &Server, method: &str) -> Result<reqwest::Url, String> {
    let mut url = validate_url(&format!("{}{method}", server.api()))?;
    url.query_pairs_mut()
        .append_pair("launcher_id", server.launcher)
        .append_pair("language", "zh-cn")
        .append_pair("game_ids[]", &server.game_id);
    Ok(url)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Branch {
    pub tag: String,
    pub branch: String,
    pub package_id: String,
    pub password: String,
}

pub fn branches(client: &Client, server: &Server) -> Result<(Branch, Option<Branch>), String> {
    let data = json(client, endpoint(server, "getGameBranches")?)?;
    let rows = data["game_branches"].as_array().ok_or("官方分支列表缺失")?;
    let row = rows
        .iter()
        .find(|v| v["game"]["id"] == server.game_id && v["game"]["biz"] == server.biz)
        .ok_or("官方分支未包含当前游戏和渠道")?;
    let main = serde_json::from_value::<Branch>(row["main"].clone())
        .map_err(|e| format!("当前版本分支无效：{e}"))?;
    version(&main.tag)?;
    let pre = if row["pre_download"].is_null() {
        None
    } else {
        let pre = serde_json::from_value::<Branch>(row["pre_download"].clone())
            .map_err(|e| format!("预下载分支无效：{e}"))?;
        version(&pre.tag)?;
        if version(&pre.tag)? > version(&main.tag)? {
            Some(pre)
        } else {
            None
        }
    };
    Ok((main, pre))
}

pub fn version(value: &str) -> Result<semver::Version, String> {
    semver::Version::parse(value).map_err(|_| format!("游戏版本无效：{value}"))
}

pub fn number(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .or_else(|| value[key].as_str()?.parse().ok())
        .ok_or_else(|| format!("缺少有效大小：{key}"))
}

#[derive(Clone, Debug, Deserialize)]
pub struct Download {
    pub url_prefix: String,
    #[serde(default)]
    pub url_suffix: String,
    pub compression: u32,
    pub encryption: u32,
}

impl Download {
    pub fn url(&self, name: &str) -> Result<String, String> {
        if self.encryption != 0
            || self.compression > 1
            || name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        {
            return Err("官方资源使用未知加密、压缩或文件标识".into());
        }
        let result = format!(
            "{}/{}{}",
            self.url_prefix.trim_end_matches('/'),
            name,
            self.url_suffix
        );
        validate_url(&result)?;
        Ok(result)
    }
}

#[derive(Clone, Debug)]
pub struct ManifestInfo {
    pub compressed: bool,
    pub checksum: String,
    pub size: u64,
    pub unpacked_size: u64,
    pub url: String,
    pub chunks: Download,
    pub matching_field: String,
}

pub fn build(
    client: &Client,
    server: &Server,
    branch: &Branch,
    languages: &[String],
) -> Result<Vec<ManifestInfo>, String> {
    let mut url = validate_url(server.downloader())?;
    url.query_pairs_mut()
        .append_pair("branch", &branch.branch)
        .append_pair("package_id", &branch.package_id)
        .append_pair("password", &branch.password)
        .append_pair("tag", &branch.tag);
    let data = json(client, url)?;
    if data["tag"] != branch.tag {
        return Err("官方 Build 与所选分支版本不一致".into());
    }
    let rows = data["manifests"].as_array().ok_or("官方文件清单缺失")?;
    let mut manifests = vec![];
    for row in rows {
        let field = row["matching_field"].as_str().ok_or("资源类别缺失")?;
        // 绝区零还有按角色等拆分的资源；完整安装包含全部非语音类别，不能只取 game。
        if field.contains('-') && !languages.iter().any(|language| language == field) {
            continue;
        }
        let manifest = &row["manifest"];
        let download: Download =
            serde_json::from_value(row["manifest_download"].clone()).map_err(|e| e.to_string())?;
        let chunks: Download =
            serde_json::from_value(row["chunk_download"].clone()).map_err(|e| e.to_string())?;
        let checksum = manifest["checksum"]
            .as_str()
            .ok_or("清单校验值缺失")?
            .to_lowercase();
        validate_md5(&checksum)?;
        let id = manifest["id"].as_str().ok_or("清单标识缺失")?;
        chunks.url("validation")?;
        manifests.push(ManifestInfo {
            compressed: download.compression == 1,
            checksum,
            size: number(manifest, "compressed_size")?,
            unpacked_size: number(manifest, "uncompressed_size")?,
            url: download.url(id)?,
            chunks,
            matching_field: field.into(),
        });
    }
    if !manifests.iter().any(|m| m.matching_field == "game")
        || languages
            .iter()
            .any(|language| !manifests.iter().any(|m| m.matching_field == *language))
    {
        return Err("当前 Build 未提供游戏本体或所选语音".into());
    }
    Ok(manifests)
}

pub fn validate_md5(value: &str) -> Result<(), String> {
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("官方 MD5 无效".into());
    }
    Ok(())
}

pub fn sdk(client: &Client, server: &Server) -> Result<Option<Value>, String> {
    let mut url = endpoint(server, "getGameChannelSDKs")?;
    url.query_pairs_mut()
        .append_pair("channel", &server.channel.to_string())
        .append_pair("sub_channel", &server.sub_channel.to_string());
    let data = json(client, url)?;
    let rows = data["game_channel_sdks"]
        .as_array()
        .ok_or("渠道 SDK 列表缺失")?;
    Ok(rows
        .iter()
        .find(|v| v["game"]["id"] == server.game_id)
        .cloned())
}

pub fn launch_config(client: &Client, server: &Server) -> Result<Value, String> {
    let data = json(client, endpoint(server, "getGameConfigs")?)?;
    let row = data["launch_configs"]
        .as_array()
        .ok_or("官方启动配置缺失")?
        .iter()
        .find(|v| v["game"]["id"] == server.game_id)
        .ok_or("官方启动配置未包含当前游戏")?;
    if row["exe_file_name"] != server.exe {
        return Err("官方游戏程序名称已经变化，请更新 SSMT".into());
    }
    Ok(row.clone())
}
