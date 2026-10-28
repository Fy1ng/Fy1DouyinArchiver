use super::contracts::{SourceCapabilityViewV1, SyncRangeKindV1, SyncSourceV1};

#[derive(Debug, Clone)]
pub struct CapabilityRegistry {
    capabilities: Vec<SourceCapabilityViewV1>,
}

impl CapabilityRegistry {
    pub fn stage_3_3() -> Self {
        let archive_ranges = vec![
            SyncRangeKindV1::All,
            SyncRangeKindV1::RecentItems,
            SyncRangeKindV1::RecordLimit,
        ];
        Self {
            capabilities: vec![
                SourceCapabilityViewV1 {
                    source: SyncSourceV1::History,
                    available: true,
                    unavailable_reason: None,
                    supported_ranges: vec![SyncRangeKindV1::Days, SyncRangeKindV1::Bottom],
                    supports_full_reconcile: false,
                    supports_remote_folders: false,
                },
                archive_capability(SyncSourceV1::Likes, archive_ranges.clone(), false),
                archive_capability(SyncSourceV1::Favorites, archive_ranges.clone(), true),
                archive_capability(SyncSourceV1::RemoteFolder, archive_ranges, false),
            ],
        }
    }

    pub fn all(&self) -> &[SourceCapabilityViewV1] {
        &self.capabilities
    }

    pub fn get(&self, source: SyncSourceV1) -> Option<&SourceCapabilityViewV1> {
        self.capabilities
            .iter()
            .find(|capability| capability.source == source)
    }

    pub fn supports_remote_folders(&self) -> bool {
        self.get(SyncSourceV1::Favorites)
            .is_some_and(|capability| capability.available)
            && self
                .get(SyncSourceV1::RemoteFolder)
                .is_some_and(|capability| capability.available)
    }
}

fn archive_capability(
    source: SyncSourceV1,
    supported_ranges: Vec<SyncRangeKindV1>,
    supports_remote_folders: bool,
) -> SourceCapabilityViewV1 {
    SourceCapabilityViewV1 {
        source,
        available: true,
        unavailable_reason: None,
        supported_ranges,
        supports_full_reconcile: true,
        supports_remote_folders,
    }
}
