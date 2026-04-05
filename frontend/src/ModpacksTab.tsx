import { marked } from 'marked';
import { faArrowDown, faCheck, faExclamationTriangle, faExternalLink, faSearch, faSpinner } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Alert,
  Badge,
  Checkbox,
  Group,
  Loader,
  Modal,
  Progress,
  SegmentedControl,
  Stack,
  Text,
  TextInput,
} from '@mantine/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Button from '@/elements/Button.tsx';
import Select from '@/elements/input/Select.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import type { ServerDetection } from './detect.ts';
import {
  CF_CLASS_MODPACKS,
  checkCurseForgeStatus,
  formatDownloads as cfFormatDownloads,
  formatSize as cfFormatSize,
  getCurseForgeDescription,
  getCurseForgeFiles,
  searchCurseForge,
  type CurseForgeFile,
  type CurseForgeProject,
} from './curseforge.ts';
import {
  formatDownloads,
  formatSize,
  getProject,
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

type Source = 'modrinth' | 'curseforge';

interface DisplayModpack {
  id: string;
  title: string;
  description: string;
  downloads: number;
  author: string;
  iconUrl: string | null;
  source: Source;
  modrinthProject?: ModrinthProject;
  curseforgeProject?: CurseForgeProject;
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

  const [source, setSource] = useState<Source>('modrinth');
  const [cfAvailable, setCfAvailable] = useState<boolean | null>(null);

  // Search state
  const [query, setQuery] = useState('');
  const [sortBy, setSortBy] = useState<string>('downloads');
  const [results, setResults] = useState<DisplayModpack[]>([]);
  const [totalHits, setTotalHits] = useState(0);
  const [loading, setLoading] = useState(false);
  const searchTimer = useRef<ReturnType<typeof setTimeout>>();

  // Install modal
  const [selectedModpack, setSelectedModpack] = useState<DisplayModpack | null>(null);
  // Modrinth versions
  const [modrinthVersions, setModrinthVersions] = useState<ModrinthVersion[]>([]);
  const [selectedModrinthVersion, setSelectedModrinthVersion] = useState<ModrinthVersion | null>(null);
  // CurseForge files
  const [cfFiles, setCfFiles] = useState<CurseForgeFile[]>([]);
  const [selectedCfFile, setSelectedCfFile] = useState<CurseForgeFile | null>(null);

  // Detail
  const [detailBody, setDetailBody] = useState<string>('');
  const [detailLoading, setDetailLoading] = useState(false);

  const [versionsLoading, setVersionsLoading] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<ModpackProgress | null>(null);
  const [cleanInstall, setCleanInstall] = useState(true);
  const [acceptRisk, setAcceptRisk] = useState(false);

  const isRunning = server.status === 'running' || server.status === 'starting';

  useEffect(() => {
    checkCurseForgeStatus(server.uuid).then(setCfAvailable);
  }, [server.uuid]);

  // Modrinth search
  const doModrinthSearch = useCallback(async (q: string, sort: string, offset: number) => {
    const res = await searchProjects({
      query: q || undefined,
      projectType: 'modpack',
      index: sort as SearchIndex,
      offset,
      limit: 20,
    });
    return {
      items: res.hits.map((p): DisplayModpack => ({
        id: p.project_id,
        title: p.title,
        description: p.description,
        downloads: p.downloads,
        author: p.author,
        iconUrl: p.icon_url,
        source: 'modrinth',
        modrinthProject: p,
      })),
      total: res.total_hits,
    };
  }, []);

  // CurseForge search
  const doCurseForgeSearch = useCallback(async (q: string, sort: string, offset: number) => {
    const sortMap: Record<string, number> = {
      relevance: 1, downloads: 6, follows: 2, newest: 11, updated: 3,
    };
    const res = await searchCurseForge(server.uuid, {
      searchFilter: q || undefined,
      classId: CF_CLASS_MODPACKS,
      sortField: sortMap[sort] ?? 6,
      sortOrder: 'desc',
      index: offset,
      pageSize: 20,
    });
    return {
      items: res.data.map((p): DisplayModpack => ({
        id: String(p.id),
        title: p.name,
        description: p.summary,
        downloads: p.downloadCount,
        author: p.authors[0]?.name ?? 'Unknown',
        iconUrl: p.logo?.thumbnailUrl ?? null,
        source: 'curseforge',
        curseforgeProject: p,
      })),
      total: res.pagination.totalCount,
    };
  }, [server.uuid]);

  const doSearch = useCallback(async (q: string, sort: string, offset: number) => {
    setLoading(true);
    try {
      const result = source === 'curseforge'
        ? await doCurseForgeSearch(q, sort, offset)
        : await doModrinthSearch(q, sort, offset);
      if (offset === 0) {
        setResults(result.items);
      } else {
        setResults((prev) => [...prev, ...result.items]);
      }
      setTotalHits(result.total);
    } catch (err) {
      addToast(`Search failed: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [source, doModrinthSearch, doCurseForgeSearch]);

  useEffect(() => {
    setResults([]);
    setTotalHits(0);
    if (searchTimer.current) clearTimeout(searchTimer.current);
    searchTimer.current = setTimeout(() => doSearch(query, sortBy, 0), 300);
    return () => { if (searchTimer.current) clearTimeout(searchTimer.current); };
  }, [query, sortBy, doSearch, source]);

  const loadMore = () => doSearch(query, sortBy, results.length);

  // Open install modal
  const openInstall = useCallback(async (modpack: DisplayModpack) => {
    setSelectedModpack(modpack);
    setVersionsLoading(true);
    setDetailLoading(true);
    setDetailBody('');
    setProgress(null);
    setCleanInstall(true);
    setAcceptRisk(false);
    setModrinthVersions([]);
    setSelectedModrinthVersion(null);
    setCfFiles([]);
    setSelectedCfFile(null);

    try {
      if (modpack.source === 'modrinth' && modpack.modrinthProject) {
        const [details, vers] = await Promise.all([
          getProject(modpack.modrinthProject.project_id),
          getProjectVersions(modpack.modrinthProject.project_id),
        ]);
        setDetailBody(details.body ?? '');
        setModrinthVersions(vers);
        const featured = vers.find((v) => v.featured) ?? vers[0];
        if (featured) setSelectedModrinthVersion(featured);
      } else if (modpack.source === 'curseforge' && modpack.curseforgeProject) {
        const [desc, res] = await Promise.all([
          getCurseForgeDescription(server.uuid, modpack.curseforgeProject.id),
          getCurseForgeFiles(server.uuid, {
            modId: modpack.curseforgeProject.id,
            pageSize: 50,
          }),
        ]);
        setDetailBody(desc);
        setCfFiles(res.data);
        if (res.data.length > 0) setSelectedCfFile(res.data[0]);
      }
    } catch (err) {
      addToast(`Failed to load versions: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setVersionsLoading(false);
      setDetailLoading(false);
    }
  }, [server.uuid]);

  // Loader info from Modrinth version
  const loaderInfo = useMemo(() => {
    if (!selectedModrinthVersion) return null;
    const loaders = selectedModrinthVersion.loaders ?? [];
    if (loaders.includes('fabric')) return { name: 'Fabric' };
    if (loaders.includes('neoforge')) return { name: 'NeoForge' };
    if (loaders.includes('forge')) return { name: 'Forge' };
    if (loaders.includes('quilt')) return { name: 'Quilt' };
    return null;
  }, [selectedModrinthVersion]);

  // Version options
  const versionOptions = useMemo(() => {
    if (selectedModpack?.source === 'modrinth') {
      return modrinthVersions.map((v) => ({
        value: v.id,
        label: `${v.version_number} (${v.game_versions.slice(0, 3).join(', ')}${v.game_versions.length > 3 ? '...' : ''})`,
      }));
    }
    return cfFiles.map((f) => ({
      value: String(f.id),
      label: `${f.displayName} (${f.gameVersions.filter((v) => /^\d/.test(v)).slice(0, 3).join(', ')})`,
    }));
  }, [selectedModpack?.source, modrinthVersions, cfFiles]);

  const hasVersions = selectedModpack?.source === 'modrinth' ? modrinthVersions.length > 0 : cfFiles.length > 0;

  // Install
  const doInstall = useCallback(async () => {
    if (!selectedModpack) return;

    setInstalling(true);
    setProgress({ state: 'preparing', total_files: 0, downloaded_files: 0, current_file: 'Starting...' });

    try {
      let endpoint: string;
      let params: URLSearchParams;

      if (selectedModpack.source === 'modrinth') {
        if (!selectedModrinthVersion) return;
        const file = getPrimaryFile(selectedModrinthVersion);
        if (!file) { addToast('No .mrpack file found.', 'error'); return; }
        endpoint = `/api/client/servers/${server.uuid}/content-installer/modpack/install`;
        params = new URLSearchParams({
          mrpack_url: file.url,
          clean_install: String(cleanInstall),
        });
      } else {
        if (!selectedCfFile) return;
        if (!selectedCfFile.downloadUrl) {
          addToast('This modpack does not allow third-party downloads.', 'error');
          return;
        }
        endpoint = `/api/client/servers/${server.uuid}/content-installer/modpack/cf-install`;
        params = new URLSearchParams({
          zip_url: selectedCfFile.downloadUrl,
          clean_install: String(cleanInstall),
        });
      }

      const res = await fetch(`${endpoint}?${params}`, { method: 'POST' });
      if (!res.ok) throw new Error(await res.text() || `Install failed: ${res.status}`);

      // Poll for progress
      const statusUrl = `/api/client/servers/${server.uuid}/content-installer/modpack/status`;
      for (let i = 0; i < 600; i++) {
        await new Promise((r) => setTimeout(r, 1500));
        const statusRes = await fetch(statusUrl);
        if (!statusRes.ok) continue;
        const status: ModpackProgress = await statusRes.json();
        setProgress(status);

        if (status.state === 'done') {
          addToast(`Modpack "${selectedModpack.title}" installed successfully!`, 'success');
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
  }, [selectedModpack, selectedModrinthVersion, selectedCfFile, cleanInstall, server.uuid]);

  const selectedFile = selectedModpack?.source === 'modrinth' && selectedModrinthVersion
    ? getPrimaryFile(selectedModrinthVersion) : null;
  const progressPct = progress && progress.total_files > 0
    ? Math.round((progress.downloaded_files / progress.total_files) * 100) : 0;

  const sourceOptions = [
    { value: 'modrinth', label: 'Modrinth' },
    ...(cfAvailable ? [{ value: 'curseforge', label: 'CurseForge' }] : []),
  ];

  const canInstall = selectedModpack?.source === 'modrinth'
    ? !!selectedModrinthVersion && !!selectedFile
    : !!selectedCfFile && !!selectedCfFile?.downloadUrl;

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
        {sourceOptions.length > 1 && (
          <SegmentedControl
            value={source}
            onChange={(v) => setSource(v as Source)}
            data={sourceOptions}
          />
        )}
        <Select
          data={[
            { value: 'relevance', label: 'Relevance' },
            { value: 'downloads', label: 'Downloads' },
            { value: 'follows', label: 'Follows' },
            { value: 'newest', label: 'Newest' },
            { value: 'updated', label: 'Updated' },
          ]}
          value={sortBy}
          onChange={(v) => v && setSortBy(v)}
          w={140}
        />
      </div>

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
            {results.map((modpack) => (
              <div
                key={`${modpack.source}-${modpack.id}`}
                className='ci-project-card'
                onClick={() => openInstall(modpack)}
              >
                <div className='ci-card-header'>
                  {modpack.iconUrl ? (
                    <img src={modpack.iconUrl} alt='' className='ci-project-icon' />
                  ) : (
                    <div className='ci-project-icon ci-project-icon--placeholder' />
                  )}
                  <div className='ci-card-title'>
                    <Text fw={600} size='sm' lineClamp={1}>{modpack.title}</Text>
                    <Text size='xs' c='dimmed'>by {modpack.author}</Text>
                  </div>
                </div>
                <div className='ci-card-body'>
                  <Text size='xs' c='dimmed' lineClamp={3}>{modpack.description}</Text>
                </div>
                <div className='ci-card-footer'>
                  <Text size='xs' c='dimmed'>
                    {(modpack.source === 'curseforge' ? cfFormatDownloads : formatDownloads)(modpack.downloads)} downloads
                  </Text>
                  <Badge variant='light' color={modpack.source === 'curseforge' ? 'orange' : 'green'} size='xs'>
                    {modpack.source === 'curseforge' ? 'CurseForge' : 'Modrinth'}
                  </Badge>
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
        opened={!!selectedModpack}
        onClose={() => { if (!installing) setSelectedModpack(null); }}
        title={null}
        size='80%'
        centered
        padding='lg'
        classNames={{ header: 'ci-modal-header', body: 'ci-modal-body' }}
        closeOnClickOutside={!installing}
        closeOnEscape={!installing}
      >
        {selectedModpack && (
          <Stack gap='md'>
            {/* Header row: icon + meta left, version + install right */}
            <div className='ci-detail-top'>
              <div className='ci-detail-top-left'>
                {selectedModpack.iconUrl ? (
                  <img src={selectedModpack.iconUrl} alt='' className='ci-detail-icon' />
                ) : (
                  <div className='ci-detail-icon ci-detail-icon--placeholder' />
                )}
                <div className='ci-detail-meta'>
                  <Group gap='xs' align='center'>
                    <Text fw={700} size='lg'>{selectedModpack.title}</Text>
                    <Badge variant='light' color={selectedModpack.source === 'curseforge' ? 'orange' : 'green'} size='xs'>
                      {selectedModpack.source === 'curseforge' ? 'CurseForge' : 'Modrinth'}
                    </Badge>
                    {selectedModpack.source === 'modrinth' && selectedModpack.modrinthProject && (
                      <Button size='compact-xs' variant='subtle' component='a'
                        href={`https://modrinth.com/modpack/${selectedModpack.modrinthProject.slug}`}
                        target='_blank' leftSection={<FontAwesomeIcon icon={faExternalLink} />}
                        onClick={(e: React.MouseEvent) => e.stopPropagation()}>View</Button>
                    )}
                    {selectedModpack.source === 'curseforge' && selectedModpack.curseforgeProject && (
                      <Button size='compact-xs' variant='subtle' component='a'
                        href={`https://www.curseforge.com/minecraft/modpacks/${selectedModpack.curseforgeProject.slug}`}
                        target='_blank' leftSection={<FontAwesomeIcon icon={faExternalLink} />}
                        onClick={(e: React.MouseEvent) => e.stopPropagation()}>View</Button>
                    )}
                  </Group>
                  <Group gap='xs'>
                    <Text size='xs' c='dimmed'>by {selectedModpack.author}</Text>
                    <Text size='xs' c='dimmed'>&middot;</Text>
                    <Text size='xs' c='dimmed'>
                      {(selectedModpack.source === 'curseforge' ? cfFormatDownloads : formatDownloads)(selectedModpack.downloads)} downloads
                    </Text>
                    {selectedModpack.source === 'modrinth' && selectedModrinthVersion && (
                      <>
                        <Text size='xs' c='dimmed'>&middot;</Text>
                        <Text size='xs' c='dimmed'>{timeAgo(selectedModrinthVersion.date_published)}</Text>
                      </>
                    )}
                    {loaderInfo && <span className='ci-tag ci-tag--violet'>{loaderInfo.name}</span>}
                  </Group>
                </div>
              </div>

              <div className='ci-detail-top-right'>
                {versionsLoading ? (
                  <Loader color='violet' size='xs' />
                ) : !hasVersions ? (
                  <Text size='xs' c='dimmed'>No versions</Text>
                ) : (
                  <Select
                    placeholder='Version...'
                    data={versionOptions}
                    value={
                      selectedModpack.source === 'modrinth'
                        ? (selectedModrinthVersion?.id ?? null)
                        : (selectedCfFile ? String(selectedCfFile.id) : null)
                    }
                    onChange={(val) => {
                      if (selectedModpack.source === 'modrinth') {
                        setSelectedModrinthVersion(modrinthVersions.find((v) => v.id === val) ?? null);
                      } else {
                        setSelectedCfFile(cfFiles.find((f) => String(f.id) === val) ?? null);
                      }
                    }}
                    searchable
                    size='sm'
                    w={220}
                    disabled={installing}
                  />
                )}
              </div>
            </div>

            {isRunning && (
              <Alert icon={<FontAwesomeIcon icon={faExclamationTriangle} />} color='red' variant='light'>
                Stop your server before installing a modpack.
              </Alert>
            )}

            {selectedModpack.source === 'curseforge' && selectedCfFile && !selectedCfFile.downloadUrl && (
              <Alert color='red' variant='light'>
                This modpack does not allow third-party downloads.
              </Alert>
            )}

            {/* Description */}
            {detailLoading ? (
              <div className='ci-center'><Loader color='violet' size='sm' /></div>
            ) : detailBody ? (
              <div
                className='ci-detail-body'
                dangerouslySetInnerHTML={{
                  __html: selectedModpack.source === 'curseforge'
                    ? detailBody
                    : (marked.parse(detailBody, { async: false, breaks: true }) as string),
                }}
              />
            ) : (
              <Text size='sm' c='dimmed'>{selectedModpack.description}</Text>
            )}

            {/* Bottom bar: checkboxes + install */}
            {hasVersions && !versionsLoading && (
              <>
                <Checkbox
                  label='Clean install (recommended)'
                  description='Wipes all existing server files before installing.'
                  checked={cleanInstall}
                  onChange={(e) => setCleanInstall(e.currentTarget.checked)}
                  color='red'
                  disabled={installing || isRunning}
                />
                <Group justify='space-between' align='center' wrap='wrap'>
                  <Checkbox
                    label='I understand this will replace my server files'
                    checked={acceptRisk}
                    onChange={(e) => setAcceptRisk(e.currentTarget.checked)}
                    disabled={installing || isRunning}
                  />
                  <Group gap='sm'>
                    {progress?.state !== 'done' && (
                      <Button
                        onClick={doInstall}
                        loading={installing}
                        disabled={isRunning || !canInstall || !acceptRisk || !hasVersions}
                        color='red'
                        leftSection={<FontAwesomeIcon icon={faArrowDown} />}
                      >
                        Install Modpack
                      </Button>
                    )}
                  </Group>
                </Group>
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
                {progress.state === 'done' && (
                  <Group justify='flex-end' mt='xs'>
                    <Button
                      onClick={() => setSelectedModpack(null)}
                      color='green'
                      leftSection={<FontAwesomeIcon icon={faCheck} />}
                    >
                      Done
                    </Button>
                  </Group>
                )}
              </div>
            )}
          </Stack>
        )}
      </Modal>
    </div>
  );
}
