import { faArrowDown, faBox, faCheck, faExclamationTriangle, faSearch, faSpinner } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Alert,
  Badge,
  Checkbox,
  Group,
  Loader,
  Modal,
  Progress,
  Stack,
  Text,
  TextInput,
  Title,
} from '@mantine/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Button from '@/elements/Button.tsx';
import Select from '@/elements/input/Select.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import type { ServerDetection } from './detect.ts';
import {
  formatDownloads,
  formatSize,
  getPrimaryFile,
  getProjectVersions,
  searchProjects,
  timeAgo,
  type ModrinthProject,
  type ModrinthVersion,
  type SearchIndex,
} from './modrinth.ts';

interface ModpacksTabProps {
  detection: ServerDetection;
}

interface ModpackProgress {
  state: string;
  total_files: number;
  downloaded_files: number;
  current_file: string;
  error?: string;
}

export default function ModpacksTab({ detection }: ModpacksTabProps) {
  const { addToast } = useToast();
  const { server } = useServerStore();

  // Search state
  const [query, setQuery] = useState('');
  const [sortBy, setSortBy] = useState<SearchIndex>('downloads');
  const [results, setResults] = useState<ModrinthProject[]>([]);
  const [totalHits, setTotalHits] = useState(0);
  const [loading, setLoading] = useState(false);
  const searchTimer = useRef<ReturnType<typeof setTimeout>>();

  // Install modal state
  const [selectedProject, setSelectedProject] = useState<ModrinthProject | null>(null);
  const [versions, setVersions] = useState<ModrinthVersion[]>([]);
  const [selectedVersion, setSelectedVersion] = useState<ModrinthVersion | null>(null);
  const [versionsLoading, setVersionsLoading] = useState(false);

  // Install progress
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<ModpackProgress | null>(null);
  const [cleanInstall, setCleanInstall] = useState(true);
  const [acceptRisk, setAcceptRisk] = useState(false);

  const isRunning = server.status === 'running' || server.status === 'starting';

  // Search
  const doSearch = useCallback(async (q: string, sort: SearchIndex, offset: number) => {
    setLoading(true);
    try {
      const res = await searchProjects({
        query: q || undefined,
        projectType: 'modpack',
        gameVersions: detection.mcVersion ? [detection.mcVersion] : undefined,
        index: sort,
        offset,
        limit: 20,
      });
      if (offset === 0) {
        setResults(res.hits);
      } else {
        setResults((prev) => [...prev, ...res.hits]);
      }
      setTotalHits(res.total_hits);
    } catch (err) {
      addToast(`Search failed: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [detection.mcVersion]);

  useEffect(() => {
    if (searchTimer.current) clearTimeout(searchTimer.current);
    searchTimer.current = setTimeout(() => doSearch(query, sortBy, 0), 300);
    return () => { if (searchTimer.current) clearTimeout(searchTimer.current); };
  }, [query, sortBy, doSearch]);

  const loadMore = () => doSearch(query, sortBy, results.length);

  // Open install modal
  const openInstall = useCallback(async (project: ModrinthProject) => {
    setSelectedProject(project);
    setSelectedVersion(null);
    setVersionsLoading(true);
    setProgress(null);
    setCleanInstall(true);
    setAcceptRisk(false);

    try {
      const vers = await getProjectVersions(project.project_id, {
        gameVersions: detection.mcVersion ? [detection.mcVersion] : undefined,
      });
      setVersions(vers);
      const featured = vers.find((v) => v.featured) ?? vers[0];
      if (featured) setSelectedVersion(featured);
    } catch (err) {
      addToast(`Failed to load versions: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setVersionsLoading(false);
    }
  }, [detection.mcVersion]);

  // Get loader info from version
  const loaderInfo = useMemo(() => {
    if (!selectedVersion) return null;
    const loaders = selectedVersion.loaders ?? [];
    // Map to MCJars type
    if (loaders.includes('fabric')) return { type: 'FABRIC', name: 'Fabric', unzip: false };
    if (loaders.includes('neoforge')) return { type: 'NEOFORGE', name: 'NeoForge', unzip: true };
    if (loaders.includes('forge')) return { type: 'FORGE', name: 'Forge', unzip: true };
    if (loaders.includes('quilt')) return { type: 'QUILT', name: 'Quilt', unzip: false };
    return null;
  }, [selectedVersion]);

  // Version options
  const versionOptions = useMemo(() =>
    versions.map((v) => ({
      value: v.id,
      label: `${v.version_number} (${v.game_versions.slice(0, 3).join(', ')}${v.game_versions.length > 3 ? '...' : ''})`,
    })),
    [versions],
  );

  // Install
  const doInstall = useCallback(async () => {
    if (!selectedVersion) return;
    const file = getPrimaryFile(selectedVersion);
    if (!file) {
      addToast('No .mrpack file found for this version.', 'error');
      return;
    }

    setInstalling(true);
    setProgress({ state: 'preparing', total_files: 0, downloaded_files: 0, current_file: 'Starting...' });

    try {
      const params = new URLSearchParams({
        mrpack_url: file.url,
        clean_install: String(cleanInstall),
      });

      if (loaderInfo) {
        params.set('loader_type', loaderInfo.type);
        params.set('loader_unzip', String(loaderInfo.unzip));
      }

      // TODO: In the future, resolve the loader jar URL from MCJars API based on the modpack's
      // required loader version. For now, the modpack overrides typically include the loader setup.

      const res = await fetch(
        `/api/client/servers/${server.uuid}/content-installer/modpack/install?${params}`,
        { method: 'POST' },
      );
      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `Install failed: ${res.status}`);
      }

      // Poll for progress
      const statusUrl = `/api/client/servers/${server.uuid}/content-installer/modpack/status`;
      for (let i = 0; i < 600; i++) { // Up to 15 minutes
        await new Promise((r) => setTimeout(r, 1500));
        const statusRes = await fetch(statusUrl);
        if (!statusRes.ok) continue;
        const status: ModpackProgress = await statusRes.json();
        setProgress(status);

        if (status.state === 'done') {
          addToast(`Modpack "${selectedProject?.title}" installed successfully!`, 'success');
          break;
        }
        if (status.state === 'error') {
          throw new Error(status.error ?? 'Installation failed');
        }
      }
    } catch (err) {
      addToast(`Modpack install failed: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
      setProgress((p) => p ? { ...p, state: 'error', error: err instanceof Error ? err.message : 'unknown' } : null);
    } finally {
      setInstalling(false);
    }
  }, [selectedVersion, selectedProject, cleanInstall, loaderInfo, server.uuid]);

  const selectedFile = selectedVersion ? getPrimaryFile(selectedVersion) : null;
  const progressPct = progress && progress.total_files > 0
    ? Math.round((progress.downloaded_files / progress.total_files) * 100)
    : 0;

  return (
    <div className='ci-browse'>
      {/* Search bar */}
      <div className='ci-search-bar'>
        <TextInput
          placeholder='Search modpacks...'
          leftSection={<FontAwesomeIcon icon={faSearch} />}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          className='ci-search-input'
        />
        <Select
          data={[
            { value: 'relevance', label: 'Relevance' },
            { value: 'downloads', label: 'Downloads' },
            { value: 'follows', label: 'Follows' },
            { value: 'newest', label: 'Newest' },
            { value: 'updated', label: 'Updated' },
          ]}
          value={sortBy}
          onChange={(v) => v && setSortBy(v as SearchIndex)}
          w={140}
        />
      </div>

      {detection.mcVersion && (
        <Group gap='xs' mt='xs' mb='sm'>
          <Badge variant='light' color='gray' size='sm'>{detection.mcVersion}</Badge>
          <Text size='xs' c='dimmed'>Showing modpacks for your Minecraft version</Text>
        </Group>
      )}

      {/* Results */}
      {loading && results.length === 0 ? (
        <div className='ci-center'><Loader color='violet' size='lg' /></div>
      ) : results.length === 0 ? (
        <Text c='dimmed' ta='center' mt='xl'>
          {query ? 'No modpacks found. Try a different search.' : 'No modpacks found.'}
        </Text>
      ) : (
        <>
          <div className='ci-results-grid'>
            {results.map((project) => (
              <div key={project.project_id} className='ci-project-card'>
                <div className='ci-project-icon-wrap'>
                  {project.icon_url ? (
                    <img src={project.icon_url} alt='' className='ci-project-icon' />
                  ) : (
                    <div className='ci-project-icon ci-project-icon--placeholder' />
                  )}
                </div>
                <div className='ci-project-info'>
                  <Text fw={600} size='sm' lineClamp={1}>{project.title}</Text>
                  <Text size='xs' c='dimmed' lineClamp={2}>{project.description}</Text>
                  <Group gap='xs' mt={4}>
                    <Text size='xs' c='dimmed'>{formatDownloads(project.downloads)} downloads</Text>
                    <Text size='xs' c='dimmed'>&middot;</Text>
                    <Text size='xs' c='dimmed'>by {project.author}</Text>
                  </Group>
                </div>
                <div className='ci-project-actions'>
                  <Button size='xs' onClick={() => openInstall(project)}>
                    Install
                  </Button>
                </div>
              </div>
            ))}
          </div>

          {results.length < totalHits && (
            <Group justify='center' mt='md'>
              <Button variant='subtle' onClick={loadMore} loading={loading}>
                Load More ({results.length}/{totalHits})
              </Button>
            </Group>
          )}
        </>
      )}

      {/* Install Modal */}
      <Modal
        opened={!!selectedProject}
        onClose={() => { if (!installing) setSelectedProject(null); }}
        title={
          <Group gap='sm'>
            {selectedProject?.icon_url && (
              <img src={selectedProject.icon_url} alt='' width={28} height={28} style={{ borderRadius: 6 }} />
            )}
            <Text fw={600}>Install {selectedProject?.title}</Text>
          </Group>
        }
        size='md'
        centered
        closeOnClickOutside={!installing}
        closeOnEscape={!installing}
      >
        {selectedProject && (
          <Stack gap='md'>
            <Text size='sm' c='dimmed'>{selectedProject.description}</Text>

            {/* Server must be stopped */}
            {isRunning && (
              <Alert icon={<FontAwesomeIcon icon={faExclamationTriangle} />} color='red' variant='light'>
                Stop your server before installing a modpack.
              </Alert>
            )}

            {versionsLoading ? (
              <div className='ci-center'><Loader color='violet' size='sm' /></div>
            ) : versions.length === 0 ? (
              <Alert color='yellow' variant='light'>
                No compatible versions found{detection.mcVersion ? ` for ${detection.mcVersion}` : ''}.
              </Alert>
            ) : (
              <>
                <Select
                  label='Version'
                  data={versionOptions}
                  value={selectedVersion?.id ?? null}
                  onChange={(val) => {
                    const v = versions.find((ver) => ver.id === val);
                    setSelectedVersion(v ?? null);
                  }}
                  searchable
                  disabled={installing}
                />

                {selectedVersion && (
                  <div className='ci-version-info'>
                    {selectedFile && (
                      <Text size='xs' c='dimmed'>
                        {selectedFile.filename} &middot; {formatSize(selectedFile.size)}
                      </Text>
                    )}
                    <Group gap='xs' mt={4}>
                      {loaderInfo && (
                        <Badge variant='light' color='violet' size='xs'>{loaderInfo.name}</Badge>
                      )}
                      {selectedVersion.game_versions.slice(0, 3).map((v) => (
                        <Badge key={v} variant='light' color='gray' size='xs'>{v}</Badge>
                      ))}
                    </Group>
                    <Text size='xs' c='dimmed' mt={4}>
                      {formatDownloads(selectedVersion.downloads)} downloads &middot; {timeAgo(selectedVersion.date_published)}
                    </Text>
                  </div>
                )}

                <Checkbox
                  label='Clean install (recommended)'
                  description='Wipes all existing server files before installing. This ensures a clean modpack setup without leftover files from previous installations.'
                  checked={cleanInstall}
                  onChange={(e) => setCleanInstall(e.currentTarget.checked)}
                  color='red'
                  disabled={installing || isRunning}
                />

                <Checkbox
                  label='I understand this will replace my server files'
                  description='Installing a modpack will overwrite existing mods, configs, and server files. Back up anything important before proceeding.'
                  checked={acceptRisk}
                  onChange={(e) => setAcceptRisk(e.currentTarget.checked)}
                  disabled={installing || isRunning}
                />
              </>
            )}

            {/* Progress */}
            {progress && progress.state !== 'idle' && (
              <div className='ci-version-info'>
                <Group gap='xs' mb='xs'>
                  {progress.state === 'done' ? (
                    <FontAwesomeIcon icon={faCheck} color='#4ade80' />
                  ) : progress.state === 'error' ? (
                    <FontAwesomeIcon icon={faExclamationTriangle} color='#ef4444' />
                  ) : (
                    <FontAwesomeIcon icon={faSpinner} spin />
                  )}
                  <Text size='sm' fw={500}>
                    {progress.state === 'preparing' && 'Preparing...'}
                    {progress.state === 'downloading_mods' && `Downloading mods (${progress.downloaded_files}/${progress.total_files})`}
                    {progress.state === 'applying_overrides' && 'Applying configs...'}
                    {progress.state === 'installing_loader' && 'Installing loader...'}
                    {progress.state === 'done' && 'Installation complete!'}
                    {progress.state === 'error' && 'Installation failed'}
                  </Text>
                </Group>
                {progress.state === 'downloading_mods' && progress.total_files > 0 && (
                  <Progress value={progressPct} color='violet' size='sm' mb='xs' />
                )}
                {progress.current_file && progress.state !== 'done' && (
                  <Text size='xs' c='dimmed'>{progress.current_file}</Text>
                )}
                {progress.error && (
                  <Text size='xs' c='red' mt='xs'>{progress.error}</Text>
                )}
              </div>
            )}

            <Group justify='flex-end' mt='sm'>
              <Button
                variant='subtle'
                onClick={() => setSelectedProject(null)}
                disabled={installing}
              >
                {progress?.state === 'done' ? 'Close' : 'Cancel'}
              </Button>
              {progress?.state !== 'done' && (
                <Button
                  onClick={doInstall}
                  loading={installing}
                  disabled={isRunning || !selectedVersion || !selectedFile || !acceptRisk || versions.length === 0}
                  color='red'
                  leftSection={<FontAwesomeIcon icon={faArrowDown} />}
                >
                  Install Modpack
                </Button>
              )}
            </Group>
          </Stack>
        )}
      </Modal>
    </div>
  );
}
