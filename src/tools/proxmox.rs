use anyhow::{Result, anyhow};
use serde_json::Value;
use std::sync::Arc;

use crate::audit::AuditLogger;
use crate::confirmation::ConfirmationManager;
use crate::connection::{ConnectionManager, ProxmoxClient};
use crate::tools::{truncate_output, wrap_output_envelope};

const OUTPUT_MAX_CHARS: usize = 10_000;
const TASK_WAIT_TIMEOUT_SECS: u64 = 120;

pub async fn node_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let nodes = client.get("/nodes").await?;

        let nodes = nodes
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from /nodes"))?;

        if nodes.is_empty() {
            return Ok("No nodes found.".to_string());
        }

        let mut lines = vec![format!(
            "Proxmox host: {}\n\n{:<16}  {:<10}  {:<8}  {:<8}  {:<12}  {}",
            host, "NODE", "STATUS", "CPU", "MEM %", "UPTIME", "PVE VERSION"
        )];

        for node in nodes {
            let name = node.get("node").and_then(Value::as_str).unwrap_or("?");
            let status = node.get("status").and_then(Value::as_str).unwrap_or("?");
            let cpu = node.get("cpu").and_then(Value::as_f64).unwrap_or(0.0);
            let maxmem = node.get("maxmem").and_then(Value::as_u64).unwrap_or(1);
            let mem = node.get("mem").and_then(Value::as_u64).unwrap_or(0);
            let mem_pct = if maxmem > 0 {
                (mem as f64 / maxmem as f64) * 100.0
            } else {
                0.0
            };
            let uptime = node.get("uptime").and_then(Value::as_u64).unwrap_or(0);
            let pve_version = node
                .get("pveversion")
                .and_then(Value::as_str)
                .unwrap_or("?");

            lines.push(format!(
                "{:<16}  {:<10}  {:<8.1}%  {:<8.1}%  {:<12}  {}",
                name,
                status,
                cpu * 100.0,
                mem_pct,
                format_uptime(uptime),
                pve_version
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.node.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.node.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.node.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn node_status(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let status = client.get(&format!("/nodes/{}/status", node_name)).await?;

        let cpu_model = status
            .pointer("/cpuinfo/model")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let cpu_cores = status
            .pointer("/cpuinfo/cpus")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cpu_sockets = status
            .pointer("/cpuinfo/sockets")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cpu_usage = status.get("cpu").and_then(Value::as_f64).unwrap_or(0.0);
        let mem_total = status
            .pointer("/memory/total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mem_used = status
            .pointer("/memory/used")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mem_free = status
            .pointer("/memory/free")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let swap_total = status
            .pointer("/swap/total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let swap_used = status
            .pointer("/swap/used")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let rootfs_total = status
            .pointer("/rootfs/total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let rootfs_used = status
            .pointer("/rootfs/used")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let uptime = status.get("uptime").and_then(Value::as_u64).unwrap_or(0);
        let pve_version = status
            .get("pveversion")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let kernel = status
            .get("kversion")
            .and_then(Value::as_str)
            .unwrap_or("?");

        let output = format!(
            "Node: {} (Proxmox host: {})\n\
             PVE Version: {}\n\
             Kernel: {}\n\
             Uptime: {}\n\n\
             CPU: {} ({} sockets, {} cores)\n\
             CPU Usage: {:.1}%\n\
             Load Average: {}\n\n\
             Memory: {} / {} ({:.1}% used)\n\
             Free: {}\n\
             Swap: {} / {}\n\n\
             Root FS: {} / {} ({:.1}% used)",
            node_name,
            host,
            pve_version,
            kernel,
            format_uptime(uptime),
            cpu_model,
            cpu_sockets,
            cpu_cores,
            cpu_usage * 100.0,
            format_loadavg(status.get("loadavg")),
            format_bytes(mem_used),
            format_bytes(mem_total),
            percent(mem_used, mem_total),
            format_bytes(mem_free),
            format_bytes(swap_used),
            format_bytes(swap_total),
            format_bytes(rootfs_used),
            format_bytes(rootfs_total),
            percent(rootfs_used, rootfs_total),
        );

        Ok(output)
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.node.status", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.node.status", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.node.status",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let filter_type = vm_type.as_deref();

        if let Some(filter_type) = filter_type {
            ensure_vm_type(filter_type)?;
        }

        let mut all_vms: Vec<Value> = Vec::new();

        if filter_type.is_none() || filter_type == Some("qemu") {
            let qemu = client.get(&format!("/nodes/{}/qemu", node_name)).await?;
            if let Some(items) = qemu.as_array() {
                for mut vm in items.clone() {
                    if let Some(object) = vm.as_object_mut() {
                        object.insert("type".to_string(), Value::String("qemu".to_string()));
                    }
                    all_vms.push(vm);
                }
            }
        }

        if filter_type.is_none() || filter_type == Some("lxc") {
            let lxc = client.get(&format!("/nodes/{}/lxc", node_name)).await?;
            if let Some(items) = lxc.as_array() {
                for mut vm in items.clone() {
                    if let Some(object) = vm.as_object_mut() {
                        object.insert("type".to_string(), Value::String("lxc".to_string()));
                    }
                    all_vms.push(vm);
                }
            }
        }

        if all_vms.is_empty() {
            return Ok(format!("No VMs/CTs found on node '{}'.", node_name));
        }

        all_vms.sort_by_key(|vm| vm.get("vmid").and_then(Value::as_u64).unwrap_or(0));

        let mut lines = vec![format!(
            "Node: {} (Proxmox host: {})\n\n{:<8}  {:<6}  {:<24}  {:<10}  {:<6}  {:<10}  {}",
            node_name, host, "VMID", "TYPE", "NAME", "STATUS", "CPU", "MEM", "UPTIME"
        )];

        for vm in &all_vms {
            let vmid = vm.get("vmid").and_then(Value::as_u64).unwrap_or(0);
            let vm_type = vm.get("type").and_then(Value::as_str).unwrap_or("?");
            let name = vm.get("name").and_then(Value::as_str).unwrap_or("?");
            let status = vm.get("status").and_then(Value::as_str).unwrap_or("?");
            let cpus = vm.get("cpus").and_then(Value::as_u64).unwrap_or(0);
            let maxmem = vm.get("maxmem").and_then(Value::as_u64).unwrap_or(0);
            let uptime = vm.get("uptime").and_then(Value::as_u64).unwrap_or(0);

            lines.push(format!(
                "{:<8}  {:<6}  {:<24}  {:<10}  {:<6}  {:<10}  {}",
                vmid,
                vm_type,
                name,
                status,
                cpus,
                format_bytes(maxmem),
                if uptime > 0 {
                    format_uptime(uptime)
                } else {
                    "-".to_string()
                }
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.vm.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.list", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.list", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_status(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(vm_type.as_deref())?;
        let path = format!("/nodes/{}/{}/{}/status/current", node_name, vm_type, vmid);
        let status = client.get(&path).await?;

        let name = status.get("name").and_then(Value::as_str).unwrap_or("?");
        let vm_status = status.get("status").and_then(Value::as_str).unwrap_or("?");
        let cpus = status.get("cpus").and_then(Value::as_u64).unwrap_or(0);
        let cpu_usage = status.get("cpu").and_then(Value::as_f64).unwrap_or(0.0);
        let maxmem = status.get("maxmem").and_then(Value::as_u64).unwrap_or(0);
        let mem = status.get("mem").and_then(Value::as_u64).unwrap_or(0);
        let maxdisk = status.get("maxdisk").and_then(Value::as_u64).unwrap_or(0);
        let disk = status.get("disk").and_then(Value::as_u64).unwrap_or(0);
        let uptime = status.get("uptime").and_then(Value::as_u64).unwrap_or(0);
        let netin = status.get("netin").and_then(Value::as_u64).unwrap_or(0);
        let netout = status.get("netout").and_then(Value::as_u64).unwrap_or(0);
        let pid = status.get("pid").and_then(Value::as_u64);
        let qmpstatus = status.get("qmpstatus").and_then(Value::as_str);

        let mut output = format!(
            "VM/CT {} ({}) on node {} (Proxmox host: {})\n\n\
             Name: {}\n\
             Type: {}\n\
             Status: {}{}\n\
             Uptime: {}\n\n\
             CPU: {} cores, {:.1}% usage\n\
             Memory: {} / {} ({:.1}% used)\n\
             Disk: {} / {} ({:.1}% used)\n\n\
             Network In: {}\n\
             Network Out: {}",
            vmid,
            vm_type,
            node_name,
            host,
            name,
            vm_type,
            vm_status,
            qmpstatus
                .map(|status| format!(" (QMP: {})", status))
                .unwrap_or_default(),
            format_uptime(uptime),
            cpus,
            cpu_usage * 100.0,
            format_bytes(mem),
            format_bytes(maxmem),
            percent(mem, maxmem),
            format_bytes(disk),
            format_bytes(maxdisk),
            percent(disk, maxdisk),
            format_bytes(netin),
            format_bytes(netout),
        );

        if let Some(pid) = pid {
            output.push_str(&format!("\nPID: {}", pid));
        }

        Ok(output)
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.status",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.status", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.status",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_start(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would start {} {} on Proxmox host '{}'.",
            vm_type, vmid, host
        );
        audit
            .log(
                "proxmox.vm.start",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.start", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let path = format!("/nodes/{}/{}/{}/status/start", node_name, vm_type, vmid);
        let response = client.post(&path, &[]).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, TASK_WAIT_TIMEOUT_SECS)
                .await?;
            Ok(format!(
                "Started {} {} on node '{}'. {}",
                vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Started {} {} on node '{}'.",
                vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.start",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.start", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.start", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_stop(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would stop {} {} on Proxmox host '{}'.",
            vm_type, vmid, host
        );
        audit
            .log("proxmox.vm.stop", &host, "dry_run", Some(&vmid.to_string()))
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.stop", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.stop",
            None,
            &format!(
                "About to STOP {} {} on Proxmox host '{}'. The VM/CT will be shut down.",
                vm_type, vmid, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.stop",
                &host,
                "confirmation_required",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(response);
    }

    vm_stop_confirmed(manager, host, node, vmid, Some(vm_type), audit).await
}

pub async fn vm_stop_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(vm_type.as_deref())?;
        let path = format!("/nodes/{}/{}/{}/status/shutdown", node_name, vm_type, vmid);
        let response = client.post(&path, &[]).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, TASK_WAIT_TIMEOUT_SECS)
                .await?;
            Ok(format!(
                "Stopped {} {} on node '{}'. {}",
                vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Stop requested for {} {} on node '{}'.",
                vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.vm.stop", &host, "success", Some(&vmid.to_string()))
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.stop", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.stop", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn vm_create(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: Option<u64>,
    vm_type: Option<String>,
    name: Option<String>,
    cores: Option<u64>,
    memory: Option<u64>,
    ostype: Option<String>,
    iso: Option<String>,
    storage: Option<String>,
    disk_size: Option<String>,
    net: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    let mut param_lines = vec![format!("Type: {}", vm_type)];
    if let Some(vmid) = vmid {
        param_lines.push(format!("VMID: {}", vmid));
    }
    if let Some(name) = &name {
        param_lines.push(format!("Name: {}", name));
    }
    if let Some(cores) = cores {
        param_lines.push(format!("Cores: {}", cores));
    }
    if let Some(memory) = memory {
        param_lines.push(format!("Memory: {} MB", memory));
    }
    if let Some(ostype) = &ostype {
        param_lines.push(format!("OS Type: {}", ostype));
    }
    if let Some(iso) = &iso {
        param_lines.push(format!("ISO: {}", iso));
    }
    if let Some(storage) = &storage {
        param_lines.push(format!("Storage: {}", storage));
    }
    if let Some(disk_size) = &disk_size {
        param_lines.push(format!("Disk: {}", disk_size));
    }
    if let Some(net) = &net {
        param_lines.push(format!("Network: {}", net));
    }

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would create {} on Proxmox host '{}':\n{}",
            vm_type,
            host,
            param_lines.join("\n")
        );
        audit
            .log("proxmox.vm.create", &host, "dry_run", name.as_deref())
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.create", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
        "name": name.clone(),
        "cores": cores,
        "memory": memory,
        "ostype": ostype.clone(),
        "iso": iso.clone(),
        "storage": storage.clone(),
        "disk_size": disk_size.clone(),
        "net": net.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.create",
            None,
            &format!(
                "About to CREATE a new {} on Proxmox host '{}':\n{}",
                vm_type,
                host,
                param_lines.join("\n")
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.create",
                &host,
                "confirmation_required",
                name.as_deref(),
            )
            .await
            .ok();
        return Ok(response);
    }

    vm_create_confirmed(
        manager, host, node, vmid, vm_type, name, cores, memory, ostype, iso, storage, disk_size,
        net, audit,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn vm_create_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: Option<u64>,
    vm_type: String,
    name: Option<String>,
    cores: Option<u64>,
    memory: Option<u64>,
    ostype: Option<String>,
    iso: Option<String>,
    storage: Option<String>,
    disk_size: Option<String>,
    net: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(Some(&vm_type))?;
        let vmid = match vmid {
            Some(vmid) => vmid,
            None => next_vmid(&client).await?,
        };

        let mut params: Vec<(&str, String)> = vec![("vmid", vmid.to_string())];

        if let Some(name) = &name {
            params.push(("name", name.clone()));
        }
        if let Some(cores) = cores {
            params.push(("cores", cores.to_string()));
        }
        if let Some(memory) = memory {
            params.push(("memory", memory.to_string()));
        }

        if vm_type == "qemu" {
            if let Some(ostype) = &ostype {
                params.push(("ostype", ostype.clone()));
            }
            if let Some(iso) = &iso {
                params.push(("ide2", format!("{},media=cdrom", iso)));
            }
            if let Some(storage) = &storage {
                let size = disk_size.as_deref().unwrap_or("32G");
                params.push(("scsi0", format!("{}:{}", storage, size)));
            }
            if let Some(net) = &net {
                params.push(("net0", net.clone()));
            }
        } else {
            if let Some(ostype) = &ostype {
                params.push(("ostemplate", ostype.clone()));
            }
            if let Some(storage) = &storage {
                let size = disk_size.as_deref().unwrap_or("8");
                params.push(("rootfs", format!("{}:{}", storage, size)));
            }
            if let Some(net) = &net {
                params.push(("net0", net.clone()));
            }
        }

        let param_refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let path = format!("/nodes/{}/{}", node_name, vm_type);
        let response = client.post(&path, &param_refs).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, TASK_WAIT_TIMEOUT_SECS)
                .await?;
            Ok(format!(
                "Created {} {} on node '{}'. {}",
                vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Created {} {} on node '{}'.",
                vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.vm.create", &host, "success", name.as_deref())
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.create", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.create",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_clone(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    newid: Option<u64>,
    name: Option<String>,
    full: Option<bool>,
    target_storage: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would clone VM {} to new VM{} on Proxmox host '{}'.\n\
             Full clone: {}\n{}",
            vmid,
            newid.map(|id| format!(" {}", id)).unwrap_or_default(),
            host,
            full.unwrap_or(true),
            name.as_ref()
                .map(|name| format!("Name: {}", name))
                .unwrap_or_default(),
        );
        audit
            .log(
                "proxmox.vm.clone",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.clone", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let newid = match newid {
            Some(newid) => newid,
            None => next_vmid(&client).await?,
        };

        let mut params: Vec<(&str, String)> = vec![("newid", newid.to_string())];
        if let Some(name) = &name {
            params.push(("name", name.clone()));
        }
        if let Some(full) = full {
            params.push(("full", bool_to_proxmox(full).to_string()));
        }
        if let Some(target_storage) = &target_storage {
            params.push(("storage", target_storage.clone()));
        }

        let param_refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let path = format!("/nodes/{}/qemu/{}/clone", node_name, vmid);
        let response = client.post(&path, &param_refs).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, TASK_WAIT_TIMEOUT_SECS)
                .await?;
            Ok(format!(
                "Cloned VM {} to {} on node '{}'. {}\n{}",
                vmid,
                newid,
                node_name,
                result,
                name.as_ref()
                    .map(|name| format!("Name: {}", name))
                    .unwrap_or_default(),
            ))
        } else {
            Ok(format!(
                "Clone requested for VM {} to {} on node '{}'.",
                vmid, newid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.clone",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.clone", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.clone", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_delete(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    purge: Option<bool>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would DELETE {} {} on Proxmox host '{}'. Purge: {}",
            vm_type,
            vmid,
            host,
            purge.unwrap_or(false)
        );
        audit
            .log(
                "proxmox.vm.delete",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.delete", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
        "purge": purge,
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.delete",
            None,
            &format!(
                "About to PERMANENTLY DELETE {} {} on Proxmox host '{}'. This cannot be undone.",
                vm_type, vmid, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.delete",
                &host,
                "confirmation_required",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(response);
    }

    vm_delete_confirmed(manager, host, node, vmid, vm_type, purge, audit).await
}

pub async fn vm_delete_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: u64,
    vm_type: String,
    purge: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(Some(&vm_type))?;
        let mut path = format!("/nodes/{}/{}/{}", node_name, vm_type, vmid);
        if purge.unwrap_or(false) {
            path.push_str("?purge=1&destroy-unreferenced-disks=1");
        }

        let response = client.delete(&path).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, TASK_WAIT_TIMEOUT_SECS)
                .await?;
            Ok(format!(
                "Deleted {} {} on node '{}'. {}",
                vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Delete requested for {} {} on node '{}'.",
                vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.delete",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.delete", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.delete",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn snapshot_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(vm_type.as_deref())?;
        let path = format!("/nodes/{}/{}/{}/snapshot", node_name, vm_type, vmid);
        let snapshots = client.get(&path).await?;

        let snapshots = snapshots
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from snapshot list"))?;

        if snapshots.is_empty()
            || (snapshots.len() == 1
                && snapshots[0].get("name").and_then(Value::as_str) == Some("current"))
        {
            return Ok(format!(
                "No snapshots found for {} {} on node '{}'.",
                vm_type, vmid, node_name
            ));
        }

        let mut lines = vec![format!(
            "Snapshots for {} {} on node '{}' (Proxmox host: {})\n\n{:<24}  {:<24}  {:<10}  {}",
            vm_type, vmid, node_name, host, "NAME", "DESCRIPTION", "RAM", "PARENT"
        )];

        for snapshot in snapshots {
            let name = snapshot.get("name").and_then(Value::as_str).unwrap_or("?");
            if name == "current" {
                continue;
            }

            let description = snapshot
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            let snaptime = snapshot.get("snaptime").and_then(Value::as_u64);
            let vmstate = snapshot.get("vmstate").and_then(Value::as_u64).unwrap_or(0);
            let parent = snapshot
                .get("parent")
                .and_then(Value::as_str)
                .unwrap_or("-");

            let time_str = snaptime
                .and_then(|timestamp| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
                })
                .map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "?".to_string());

            lines.push(format!(
                "{:<24}  {:<24}  {:<10}  {}\n  Created: {}",
                name,
                truncate_cell(description, 24),
                if vmstate > 0 { "yes" } else { "no" },
                parent,
                time_str,
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.snapshot.list",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.snapshot.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.snapshot.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn snapshot_create(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    snapname: String,
    description: Option<String>,
    vmstate: Option<bool>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would create snapshot '{}' for {} {} on Proxmox host '{}'.",
            snapname, vm_type, vmid, host
        );
        audit
            .log(
                "proxmox.vm.snapshot.create",
                &host,
                "dry_run",
                Some(&snapname),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.snapshot.create", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;

        let mut params: Vec<(&str, String)> = vec![("snapname", snapname.clone())];
        if let Some(description) = &description {
            params.push(("description", description.clone()));
        }
        if let Some(vmstate) = vmstate {
            params.push(("vmstate", bool_to_proxmox(vmstate).to_string()));
        }

        let param_refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let path = format!("/nodes/{}/{}/{}/snapshot", node_name, vm_type, vmid);
        let response = client.post(&path, &param_refs).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, TASK_WAIT_TIMEOUT_SECS)
                .await?;
            Ok(format!(
                "Created snapshot '{}' for {} {} on node '{}'. {}",
                snapname, vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Snapshot '{}' created for {} {} on node '{}'.",
                snapname, vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.snapshot.create",
                    &host,
                    "success",
                    Some(&snapname),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.snapshot.create", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.snapshot.create",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn snapshot_rollback(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    snapname: String,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would rollback {} {} to snapshot '{}' on Proxmox host '{}'.",
            vm_type, vmid, snapname, host
        );
        audit
            .log(
                "proxmox.vm.snapshot.rollback",
                &host,
                "dry_run",
                Some(&snapname),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope(
            "proxmox.vm.snapshot.rollback",
            &output,
        ));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
        "snapname": snapname.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.snapshot.rollback",
            None,
            &format!(
                "About to ROLLBACK {} {} to snapshot '{}' on Proxmox host '{}'. Current state will be lost.",
                vm_type, vmid, snapname, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.snapshot.rollback",
                &host,
                "confirmation_required",
                Some(&snapname),
            )
            .await
            .ok();
        return Ok(response);
    }

    snapshot_rollback_confirmed(manager, host, node, vmid, vm_type, snapname, audit).await
}

pub async fn snapshot_rollback_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: u64,
    vm_type: String,
    snapname: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(Some(&vm_type))?;
        let path = format!(
            "/nodes/{}/{}/{}/snapshot/{}/rollback",
            node_name, vm_type, vmid, snapname
        );
        let response = client.post(&path, &[]).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, TASK_WAIT_TIMEOUT_SECS)
                .await?;
            Ok(format!(
                "Rolled back {} {} to snapshot '{}' on node '{}'. {}",
                vm_type, vmid, snapname, node_name, result
            ))
        } else {
            Ok(format!(
                "Rollback requested for {} {} to snapshot '{}' on node '{}'.",
                vm_type, vmid, snapname, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.snapshot.rollback",
                    &host,
                    "success",
                    Some(&snapname),
                )
                .await
                .ok();
            Ok(wrap_output_envelope(
                "proxmox.vm.snapshot.rollback",
                &output,
            ))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.snapshot.rollback",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn storage_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let storage = client.get(&format!("/nodes/{}/storage", node_name)).await?;

        let storage = storage
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from storage list"))?;

        if storage.is_empty() {
            return Ok(format!("No storage pools found on node '{}'.", node_name));
        }

        let mut lines = vec![format!(
            "Storage on node '{}' (Proxmox host: {})\n\n{:<20}  {:<10}  {:<10}  {:<12}  {:<12}  {:<8}  {}",
            node_name,
            host,
            "STORAGE",
            "TYPE",
            "STATUS",
            "USED",
            "TOTAL",
            "USE%",
            "CONTENT"
        )];

        for pool in storage {
            let name = pool.get("storage").and_then(Value::as_str).unwrap_or("?");
            let pool_type = pool.get("type").and_then(Value::as_str).unwrap_or("?");
            let active = pool.get("active").and_then(Value::as_u64).unwrap_or(0);
            let used = pool.get("used").and_then(Value::as_u64).unwrap_or(0);
            let total = pool.get("total").and_then(Value::as_u64).unwrap_or(0);
            let used_fraction = pool
                .get("used_fraction")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let content = pool.get("content").and_then(Value::as_str).unwrap_or("?");

            lines.push(format!(
                "{:<20}  {:<10}  {:<10}  {:<12}  {:<12}  {:<8.1}%  {}",
                name,
                pool_type,
                if active == 1 { "active" } else { "inactive" },
                format_bytes(used),
                format_bytes(total),
                used_fraction * 100.0,
                content,
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.storage.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.storage.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.storage.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn network_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| default_proxmox_host(&manager));
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let networks = client.get(&format!("/nodes/{}/network", node_name)).await?;

        let networks = networks
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from network list"))?;

        if networks.is_empty() {
            return Ok(format!("No network interfaces found on node '{}'.", node_name));
        }

        let mut lines = vec![format!(
            "Network interfaces on node '{}' (Proxmox host: {})\n\n{:<16}  {:<10}  {:<18}  {:<10}  {:<8}  {}",
            node_name, host, "IFACE", "TYPE", "ADDRESS", "ACTIVE", "AUTO", "BRIDGE PORTS"
        )];

        for iface in networks {
            let name = iface.get("iface").and_then(Value::as_str).unwrap_or("?");
            let iface_type = iface.get("type").and_then(Value::as_str).unwrap_or("?");
            let address = iface.get("address").and_then(Value::as_str).unwrap_or("-");
            let cidr = iface.get("cidr").and_then(Value::as_str);
            let active = iface.get("active").and_then(Value::as_u64).unwrap_or(0);
            let autostart = iface.get("autostart").and_then(Value::as_u64).unwrap_or(0);
            let bridge_ports = iface
                .get("bridge_ports")
                .and_then(Value::as_str)
                .unwrap_or("-");

            lines.push(format!(
                "{:<16}  {:<10}  {:<18}  {:<10}  {:<8}  {}",
                name,
                iface_type,
                cidr.unwrap_or(address),
                if active == 1 { "yes" } else { "no" },
                if autostart == 1 { "yes" } else { "no" },
                bridge_ports,
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.network.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.network.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.network.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

fn default_proxmox_host(manager: &ConnectionManager) -> String {
    manager
        .config()
        .proxmox
        .hosts
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "default".to_string())
}

fn ensure_vm_type(vm_type: &str) -> Result<()> {
    match vm_type {
        "qemu" | "lxc" => Ok(()),
        other => Err(anyhow!("vm_type must be 'qemu' or 'lxc'. Got '{}'", other)),
    }
}

fn resolved_vm_type(vm_type: Option<&str>) -> Result<&str> {
    let vm_type = vm_type.unwrap_or("qemu");
    ensure_vm_type(vm_type)?;
    Ok(vm_type)
}

async fn next_vmid(client: &ProxmoxClient) -> Result<u64> {
    let next = client.get("/cluster/nextid").await?;
    next.as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| next.as_u64())
        .ok_or_else(|| anyhow!("Failed to get next available VMID"))
}

fn bool_to_proxmox(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn percent(used: u64, total: u64) -> f64 {
    if total > 0 {
        (used as f64 / total as f64) * 100.0
    } else {
        0.0
    }
}

fn format_loadavg(loadavg: Option<&Value>) -> String {
    match loadavg.and_then(Value::as_array) {
        Some(values) if !values.is_empty() => values
            .iter()
            .map(|value| match value {
                Value::String(value) => value.clone(),
                Value::Number(value) => value.to_string(),
                _ => "?".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => "?".to_string(),
    }
}

fn truncate_cell(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }

    value.chars().take(max_chars).collect()
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;

    if days > 0 {
        format!("{}d {}h {}m", days, hours, minutes)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
