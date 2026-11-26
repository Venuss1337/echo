//! Windows network identity used at the connector boundary.
use std::{
    collections::HashSet,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ptr,
};
use windows::{
    Win32::{
        NetworkManagement::{
            Dns::{
                DNS_QUERY_STANDARD, DNS_RECORDW, DNS_TYPE_SRV, DnsFree, DnsFreeRecordList,
                DnsQuery_W,
            },
            IpHelper::{FreeMibTable, GetUnicastIpAddressTable},
        },
        Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC},
    },
    core::w,
};

pub(super) fn local_addresses() -> io::Result<HashSet<IpAddr>> {
    let mut table = ptr::null_mut();
    // SAFETY: Windows initializes the output pointer on success. The padded
    // first row is located by the binding, and NumEntries bounds the array.
    unsafe {
        if GetUnicastIpAddressTable(AF_UNSPEC, &mut table).0 != 0 || table.is_null() {
            return Err(io::Error::other("local interface inventory unavailable"));
        }
        let count = (*table).NumEntries as usize;
        let first = ptr::addr_of!((*table).Table).cast::<_>();
        let addresses = std::slice::from_raw_parts(first, count)
            .iter()
            .filter_map(
                |row: &windows::Win32::NetworkManagement::IpHelper::MIB_UNICASTIPADDRESS_ROW| {
                    let address = row.Address;
                    match address.si_family {
                        AF_INET => Some(IpAddr::V4(Ipv4Addr::from(
                            address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes(),
                        ))),
                        AF_INET6 => Some(IpAddr::V6(Ipv6Addr::from(address.Ipv6.sin6_addr.u.Byte))),
                        _ => None,
                    }
                },
            )
            .map(|ip| ip.to_canonical())
            .collect();
        FreeMibTable(table.cast());
        Ok(addresses)
    }
}

pub(super) fn directory_mirrors() -> Vec<String> {
    let mut list = ptr::null_mut();
    let mut mirrors = Vec::new();
    // SAFETY: DnsQuery_W returns UTF-16 DNS_RECORDW entries despite the
    // encoding-independent output declaration. Keep the owner until traversal
    // finishes and inspect the union only for records of the requested type.
    unsafe {
        let result = DnsQuery_W(
            w!("_api._tcp.radio-browser.info."),
            DNS_TYPE_SRV,
            DNS_QUERY_STANDARD,
            None,
            &mut list,
            None,
        );
        if result.0 == 0 {
            let mut next = list.cast::<DNS_RECORDW>();
            for _ in 0..32 {
                let Some(record) = next.as_ref() else { break };
                next = record.pNext;
                if record.wType != DNS_TYPE_SRV.0 {
                    continue;
                }
                let target = record.Data.SRV.pNameTarget;
                if target.is_null() {
                    continue;
                }
                if let Ok(name) = target.to_string() {
                    let name = name.trim_end_matches('.').to_ascii_lowercase();
                    if mirror_name(&name) && !mirrors.contains(&name) {
                        mirrors.push(name);
                    }
                }
            }
        }
        if !list.is_null() {
            DnsFree(Some(list.cast()), DnsFreeRecordList);
        }
    }
    mirrors
}

pub(super) fn mirror_name(host: &str) -> bool {
    host.strip_suffix(".api.radio-browser.info")
        .is_some_and(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}
