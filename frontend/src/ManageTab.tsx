import { faArrowUp, faExternalLink, faTrash } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Alert,
  Badge,
  Group,
  Loader,
  Modal,
  Stack,
  Text,
} from '@mantine/core';
import { useCallback, useEffect, useState } from 'react';
import { axiosInstance } from '@/api/axios.ts';
import Button from '@/elements/Button.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import type { ServerDetection } from './detect.ts';
import { LOADER_TO_MODRINTH } from './detect.ts';
import {
  formatDownloads,
  formatSize,
  getPrimaryFile,
  getProjects,
  getProjectVersions,
  getVersionsFromHashes,
  type ModrinthProjectDetails,
  type ModrinthVersion,
} from './modrinth.ts';

interface ManageTabProps {
  detection: ServerDetection;
  contentType: 'plugins' | 'mods' | 'datapacks';
  installDir: string;
  refreshKey: number;
}

interface InstalledFile {
  name: string;
  size: number;
  modified: string;
}

interface IdentifiedContent {
  file: InstalledFile;
  version: ModrinthVersion | null;
  project: ModrinthProjectDetails | null;
  latestVersion: ModrinthVersion | null;
  hasUpdate: boolean;
}

export default function ManageTab({ detection, contentType, installDir, refreshKey }: ManageTabProps) {
  const { addToast } = useToast();
  const { server } = useServerStore();

  const [loading, setLoading] = useState(true);
  const [items, setItems] = useState<IdentifiedContent[]>([]);

  // Action states
  const [removing, setRemoving] = useState<string | null>(null);
  const [updating, setUpdating] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<IdentifiedContent | null>(null);

  const loaders = LOADER_TO_MODRINTH[detection.loader] ?? [];

  const loadInstalled = useCallback(async () => {
    setLoading(true);
    try {
      // List files in plugins/ or mods/ directory
      const { data } = await axiosInstance.get(`/api/client/servers/${server.uuid}/files/list`, {
        params: { directory: `/${installDir}`, page: 1, per_page: 200, sort: 'name_asc' },
      });

      const entries = (data.entries?.data ?? []) as Array<{
        name: string; size: number; file: boolean; directory: boolean; modified: string;
      }>;

      // Filter to relevant files (jars for plugins/mods, zips for datapacks)
      const validExts = contentType === 'datapacks' ? ['.zip'] : ['.jar'];
      const jarFiles: InstalledFile[] = entries
        .filter((e) => e.file && validExts.some((ext) => e.name.endsWith(ext)))
        .map((e) => ({ name: e.name, size: e.size, modified: e.modified }));

      if (jarFiles.length === 0) {
        setItems([]);
        setLoading(false);
        return;
      }

      // Try to identify files via Modrinth hash lookup
      // We need SHA-1 hashes, but we can't hash files from the frontend.
      // Instead, build items without hash identification for now.
      // We'll show all jar files and let users see what's installed.
      const identified: IdentifiedContent[] = jarFiles.map((file) => ({
        file,
        version: null,
        project: null,
        latestVersion: null,
        hasUpdate: false,
      }));

      setItems(identified);
    } catch (err) {
      // Directory might not exist yet
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, [server.uuid, contentType]);

  useEffect(() => {
    loadInstalled();
  }, [loadInstalled, refreshKey]);

  // Remove a file
  const doRemove = useCallback(async (item: IdentifiedContent) => {
    setRemoving(item.file.name);
    setConfirmRemove(null);
    try {
      const params = new URLSearchParams({
        filename: item.file.name,
        directory: installDir,
      });
      const res = await fetch(
        `/api/client/servers/${server.uuid}/content-installer/remove?${params}`,
        { method: 'POST' },
      );
      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `Remove failed: ${res.status}`);
      }
      addToast(`Removed ${item.project?.title ?? item.file.name}`, 'success');
      setItems((prev) => prev.filter((i) => i.file.name !== item.file.name));
    } catch (err) {
      addToast(`Failed to remove: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setRemoving(null);
    }
  }, [contentType, server.uuid]);

  // Update a file (download latest version, replacing old file)
  const doUpdate = useCallback(async (item: IdentifiedContent) => {
    if (!item.latestVersion) return;
    const file = getPrimaryFile(item.latestVersion);
    if (!file) return;

    setUpdating(item.file.name);
    try {
      // Remove old file first
      const removeParams = new URLSearchParams({
        filename: item.file.name,
        directory: installDir,
      });
      await fetch(
        `/api/client/servers/${server.uuid}/content-installer/remove?${removeParams}`,
        { method: 'POST' },
      );

      // Install new version
      const installParams = new URLSearchParams({
        url: file.url,
        filename: file.filename,
        directory: installDir,
      });
      const res = await fetch(
        `/api/client/servers/${server.uuid}/content-installer/install?${installParams}`,
        { method: 'POST' },
      );
      if (!res.ok) throw new Error(await res.text());

      // Poll until done
      const statusUrl = `/api/client/servers/${server.uuid}/content-installer/install/status`;
      for (let i = 0; i < 60; i++) {
        await new Promise((r) => setTimeout(r, 1500));
        const statusRes = await fetch(statusUrl);
        if (!statusRes.ok) break;
        const status = await statusRes.json();
        if (status.state === 'done') break;
      }

      addToast(`Updated ${item.project?.title ?? item.file.name}!`, 'success');
      loadInstalled();
    } catch (err) {
      addToast(`Update failed: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setUpdating(null);
    }
  }, [contentType, server.uuid, loadInstalled]);

  const isRunning = server.status === 'running' || server.status === 'starting';

  return (
    <div className='ci-manage'>
      {isRunning && (
        <Alert color='yellow' variant='light' mb='md'>
          Server is running. Restart after making changes.
        </Alert>
      )}

      {loading ? (
        <div className='ci-center'><Loader color='violet' size='lg' /></div>
      ) : items.length === 0 ? (
        <Text c='dimmed' ta='center' mt='xl'>
          No {contentType === 'plugins' ? 'plugins' : 'mods'} installed.
          Browse and install some from the Browse tab!
        </Text>
      ) : (
        <div className='ci-installed-grid'>
          {items.map((item) => (
            <div key={item.file.name} className='ci-installed-card'>
              <div className='ci-installed-icon-wrap'>
                {item.project?.icon_url ? (
                  <img src={item.project.icon_url} alt='' className='ci-project-icon' />
                ) : (
                  <div className='ci-project-icon ci-project-icon--placeholder' />
                )}
              </div>
              <div className='ci-installed-info'>
                <Text fw={600} size='sm' lineClamp={1}>
                  {item.project?.title ?? item.file.name.replace('.jar', '')}
                </Text>
                <Text size='xs' c='dimmed'>
                  {item.file.name} &middot; {formatSize(item.file.size)}
                </Text>
                {item.version && (
                  <Text size='xs' c='dimmed'>v{item.version.version_number}</Text>
                )}
              </div>
              <div className='ci-installed-actions'>
                {item.hasUpdate && item.latestVersion && (
                  <Button
                    size='xs'
                    color='green'
                    variant='light'
                    loading={updating === item.file.name}
                    leftSection={<FontAwesomeIcon icon={faArrowUp} />}
                    onClick={() => doUpdate(item)}
                  >
                    Update
                  </Button>
                )}
                {item.project && (
                  <Button
                    size='xs'
                    variant='subtle'
                    component='a'
                    href={`https://modrinth.com/${item.project.project_type}/${item.project.slug}`}
                    target='_blank'
                    leftSection={<FontAwesomeIcon icon={faExternalLink} />}
                  >
                    Details
                  </Button>
                )}
                <Button
                  size='xs'
                  color='red'
                  variant='light'
                  loading={removing === item.file.name}
                  leftSection={<FontAwesomeIcon icon={faTrash} />}
                  onClick={() => setConfirmRemove(item)}
                >
                  Remove
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Confirm remove modal */}
      <Modal
        opened={!!confirmRemove}
        onClose={() => setConfirmRemove(null)}
        title={<Text fw={600}>Remove {confirmRemove?.project?.title ?? confirmRemove?.file.name}</Text>}
        size='sm'
        centered
      >
        {confirmRemove && (
          <Stack gap='md'>
            <Text size='sm'>
              This will delete <strong>{confirmRemove.file.name}</strong> from the {contentType} directory.
              This cannot be undone.
            </Text>
            <Group justify='flex-end'>
              <Button variant='subtle' onClick={() => setConfirmRemove(null)}>Cancel</Button>
              <Button color='red' onClick={() => doRemove(confirmRemove)}>
                Remove
              </Button>
            </Group>
          </Stack>
        )}
      </Modal>
    </div>
  );
}
