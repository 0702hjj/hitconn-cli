use anyhow::Result;
use hitconn_core::Client;
use hitconn_core::auth::AuthSession;

use crate::auth;
use crate::paths::StatePaths;

pub async fn print(paths: &StatePaths, search: Option<&str>, json: bool) -> Result<()> {
    let snapshot = paths.load_session()?;
    let mut session = AuthSession::from_snapshot(auth::gateway_config()?, snapshot)?;
    session.resume().await?;
    let client = Client::from_authenticated(session).await?;
    let mut rows = client.resources().authorized_resources();
    if let Some(query) = search.map(str::trim).filter(|query| !query.is_empty()) {
        let query = query.to_ascii_lowercase();
        rows.retain(|row| {
            [
                row.name.as_str(),
                row.target.as_str(),
                row.protocol,
                row.kind,
            ]
            .into_iter()
            .any(|value| value.to_ascii_lowercase().contains(&query))
                || ports(row.port_start, row.port_end).contains(&query)
        });
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("Authorized resources: {}", rows.len());
        for row in rows {
            println!(
                "{}\t{}\t{}/{}\t{}",
                row.name,
                row.target,
                row.protocol,
                ports(row.port_start, row.port_end),
                row.kind
            );
        }
    }
    Ok(())
}

fn ports(start: u16, end: u16) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}
