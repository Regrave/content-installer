import { marked } from 'marked';
import { faArrowDown, faCheck, faExternalLink, faRefresh, faSearch } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Group,
  Loader,
  Stack,
  Text,
} from '@mantine/core';
import SegmentedControl from '@/elements/SegmentedControl.tsx';
import { faArrowUp } from '@fortawesome/free-solid-svg-icons';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { axiosInstance } from '@/api/axios.ts';
import Alert from '@/elements/Alert.tsx';
import Badge from '@/elements/Badge.tsx';
import Button from '@/elements/Button.tsx';
import Card from '@/elements/Card.tsx';
import { Modal } from '@/elements/modals/Modal.tsx';
import Select from '@/elements/input/Select.tsx';
import TextInput from '@/elements/input/TextInput.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import type { ServerDetection } from './detect.ts';
import { LOADER_TO_CURSEFORGE, LOADER_TO_MODRINTH } from './detect.ts';
import {
  CF_CLASS_DATAPACKS,
  CF_CLASS_MODS,
  CF_CLASS_PLUGINS,
  checkCurseForgeStatus,
  formatDownloads as cfFormatDownloads,
  formatSize as cfFormatSize,
  getCurseForgeDescription,
  getCurseForgeFiles,
  releaseTypeLabel,
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
  type ModrinthProjectDetails,
  type ModrinthVersion,
  type SearchIndex,
} from './modrinth.ts';

interface BrowseTabProps {
  detection: ServerDetection;
  contentType: 'plugins' | 'mods' | 'datapacks';
  installDir: string;
  onInstalled?: () => void;
}

type Source = 'modrinth' | 'curseforge';
type InstallState = 'idle' | 'downloading' | 'done' | 'error';

interface DisplayProject {
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

export default function BrowseTab({ detection, contentType, installDir, onInstalled }: BrowseTabProps) {
  const { addToast } = useToast();
  const { server } = useServerStore();

  const [source, setSource] = useState<Source>('modrinth');
  const [cfAvailable, setCfAvailable] = useState<boolean | null>(null);

  // Search state
  const [query, setQuery] = useState('');
  const [sortBy, setSortBy] = useState<string>('relevance');
  const [results, setResults] = useState<DisplayProject[]>([]);
  const [totalHits, setTotalHits] = useState(0);
  const [loading, setLoading] = useState(false);
  const searchTimer = useRef<ReturnType<typeof setTimeout>>();

  // Detail modal
  const [selectedProject, setSelectedProject] = useState<DisplayProject | null>(null);
  const [detailBody, setDetailBody] = useState<string>('');
  const [detailLoading, setDetailLoading] = useState(false);

  // Install state within detail modal
  const [modrinthVersions, setModrinthVersions] = useState<ModrinthVersion[]>([]);
  const [selectedModrinthVersion, setSelectedModrinthVersion] = useState<ModrinthVersion | null>(null);
  const [cfFiles, setCfFiles] = useState<CurseForgeFile[]>([]);
  const [selectedCfFile, setSelectedCfFile] = useState<CurseForgeFile | null>(null);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [installState, setInstallState] = useState<InstallState>('idle');
  const [installing, setInstalling] = useState(false);
  const [existingFile, setExistingFile] = useState<string | null>(null);

  const modrinthLoaders = contentType === 'datapacks' ? ['datapack'] : (LOADER_TO_MODRINTH[detection.loader] ?? []);
  const cfModLoaderType = LOADER_TO_CURSEFORGE[detection.loader] ?? 0;
  const cfClassId = contentType === 'datapacks' ? CF_CLASS_DATAPACKS
    : contentType === 'plugins' ? CF_CLASS_PLUGINS : CF_CLASS_MODS;

  useEffect(() => {
    checkCurseForgeStatus(server.uuid).then(setCfAvailable);
  }, [server.uuid]);

  // Modrinth search
  const doModrinthSearch = useCallback(async (q: string, sort: string, offset: number) => {
    const projectType = contentType === 'datapacks' ? 'datapack' as const
      : contentType === 'plugins' ? 'plugin' as const : 'mod' as const;
    const res = await searchProjects({
      query: q || undefined,
      projectType,
      loaders: modrinthLoaders.length > 0 ? modrinthLoaders : undefined,
      gameVersions: detection.mcVersion ? [detection.mcVersion] : undefined,
      index: sort as SearchIndex,
      offset,
      limit: 20,
    });
    const items: DisplayProject[] = res.hits.map((p) => ({
      id: p.project_id,
      title: p.title,
      description: p.description,
      downloads: p.downloads,
      author: p.author,
      iconUrl: p.icon_url,
      source: 'modrinth' as const,
      modrinthProject: p,
    }));
    return { items, total: res.total_hits };
  }, [contentType, modrinthLoaders, detection.mcVersion]);

  // CurseForge search
  const doCurseForgeSearch = useCallback(async (q: string, sort: string, offset: number) => {
    const sortMap: Record<string, number> = {
      relevance: 1, downloads: 6, follows: 2, newest: 11, updated: 3,
    };
    const res = await searchCurseForge(server.uuid, {
      searchFilter: q || undefined,
      classId: cfClassId,
      gameVersion: detection.mcVersion ?? undefined,
      modLoaderType: cfModLoaderType > 0 ? cfModLoaderType : undefined,
      sortField: sortMap[sort] ?? 1,
      sortOrder: 'desc',
      index: offset,
      pageSize: 20,
    });
    const items: DisplayProject[] = res.data.map((p) => ({
      id: String(p.id),
      title: p.name,
      description: p.summary,
      downloads: p.downloadCount,
      author: p.authors[0]?.name ?? 'Unknown',
      iconUrl: p.logo?.thumbnailUrl ?? null,
      source: 'curseforge' as const,
      curseforgeProject: p,
    }));
    return { items, total: res.pagination.totalCount };
  }, [server.uuid, cfClassId, detection.mcVersion, cfModLoaderType]);

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
      addToast(`Search failed: ${err instanceof Error ? err.message : 'unknown error'}`, 'error');
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

  // Check for existing file matching a project slug in the install directory
  const checkExistingFile = useCallback(async (slug: string): Promise<string | null> => {
    try {
      const { data } = await axiosInstance.get(`/api/client/servers/${server.uuid}/files/list`, {
        params: { directory: `/${installDir}`, page: 1, per_page: 100, sort: 'name_asc' },
      });
      const entries = (data.entries?.data ?? []) as Array<{ name: string; file: boolean }>;
      const slugLower = slug.toLowerCase().replace(/[^a-z0-9]/g, '');
      const match = entries.find((e) => {
        if (!e.file) return false;
        const nameLower = e.name.toLowerCase().replace(/[^a-z0-9.]/g, '');
        return nameLower.startsWith(slugLower) || nameLower.includes(slugLower);
      });
      return match?.name ?? null;
    } catch {
      return null;
    }
  }, [server.uuid, installDir]);

  // Open detail modal
  const openDetail = useCallback(async (project: DisplayProject) => {
    setSelectedProject(project);
    setDetailBody('');
    setDetailLoading(true);
    setInstallState('idle');
    setModrinthVersions([]);
    setSelectedModrinthVersion(null);
    setCfFiles([]);
    setSelectedCfFile(null);
    setVersionsLoading(true);
    setExistingFile(null);

    // Get the slug for duplicate detection
    const slug = project.source === 'modrinth'
      ? project.modrinthProject?.slug
      : project.curseforgeProject?.slug;

    try {
      // Fetch description, versions, and existing file check in parallel
      const existingPromise = slug ? checkExistingFile(slug) : Promise.resolve(null);

      if (project.source === 'modrinth' && project.modrinthProject) {
        const [details, vers, existing] = await Promise.all([
          getProject(project.modrinthProject.project_id),
          getProjectVersions(project.modrinthProject.project_id, {
            loaders: modrinthLoaders.length > 0 ? modrinthLoaders : undefined,
            gameVersions: detection.mcVersion ? [detection.mcVersion] : undefined,
          }),
          existingPromise,
        ]);
        setDetailBody(details.body ?? '');
        setModrinthVersions(vers);
        setExistingFile(existing);
        const featured = vers.find((v) => v.featured) ?? vers[0];
        if (featured) setSelectedModrinthVersion(featured);
      } else if (project.source === 'curseforge' && project.curseforgeProject) {
        const [desc, filesRes, existing] = await Promise.all([
          getCurseForgeDescription(server.uuid, project.curseforgeProject.id),
          getCurseForgeFiles(server.uuid, {
            modId: project.curseforgeProject.id,
            gameVersion: detection.mcVersion ?? undefined,
            modLoaderType: cfModLoaderType > 0 ? cfModLoaderType : undefined,
            pageSize: 50,
          }),
          existingPromise,
        ]);
        setDetailBody(desc);
        setCfFiles(filesRes.data);
        setExistingFile(existing);
        if (filesRes.data.length > 0) setSelectedCfFile(filesRes.data[0]);
      }
    } catch (err) {
      addToast(`Failed to load details: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setDetailLoading(false);
      setVersionsLoading(false);
    }
  }, [server.uuid, modrinthLoaders, detection.mcVersion, cfModLoaderType, checkExistingFile]);

  // Install
  const doInstall = useCallback(async () => {
    if (!selectedProject) return;

    let downloadUrl: string;
    let filename: string;

    if (selectedProject.source === 'modrinth') {
      if (!selectedModrinthVersion) return;
      const file = getPrimaryFile(selectedModrinthVersion);
      if (!file) { addToast('No downloadable file found.', 'error'); return; }
      downloadUrl = file.url;
      filename = file.filename;
    } else {
      if (!selectedCfFile) return;
      if (!selectedCfFile.downloadUrl) {
        addToast('This mod does not allow third-party downloads.', 'error');
        return;
      }
      downloadUrl = selectedCfFile.downloadUrl;
      filename = selectedCfFile.fileName;
    }

    setInstalling(true);
    setInstallState('downloading');

    try {
      // Remove existing version first if updating
      if (existingFile) {
        const removeParams = new URLSearchParams({ filename: existingFile, directory: installDir });
        await fetch(
          `/api/client/servers/${server.uuid}/content-installer/remove?${removeParams}`,
          { method: 'POST' },
        );
      }

      const params = new URLSearchParams({ url: downloadUrl, filename, directory: installDir });
      const res = await fetch(
        `/api/client/servers/${server.uuid}/content-installer/install?${params}`,
        { method: 'POST' },
      );
      if (!res.ok) throw new Error(await res.text() || `Install failed: ${res.status}`);

      const statusUrl = `/api/client/servers/${server.uuid}/content-installer/install/status`;
      for (let i = 0; i < 60; i++) {
        await new Promise((r) => setTimeout(r, 1500));
        const statusRes = await fetch(statusUrl);
        if (!statusRes.ok) break;
        const status = await statusRes.json();
        if (status.state === 'done') break;
      }

      setInstallState('done');
      addToast(`Installed ${selectedProject.title ?? filename}!`, 'success');
      onInstalled?.();
    } catch (err) {
      setInstallState('error');
      addToast(`Install failed: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setInstalling(false);
    }
  }, [selectedProject, selectedModrinthVersion, selectedCfFile, installDir, server.uuid]);

  // Version selector options
  const versionOptions = useMemo(() => {
    if (selectedProject?.source === 'modrinth') {
      return modrinthVersions.map((v) => ({
        value: v.id,
        label: `${v.version_number} (${v.game_versions.slice(0, 3).join(', ')}${v.game_versions.length > 3 ? '...' : ''})`,
      }));
    }
    return cfFiles.map((f) => ({
      value: String(f.id),
      label: `${f.displayName} (${f.gameVersions.filter((v) => /^\d/.test(v)).slice(0, 3).join(', ')})`,
    }));
  }, [selectedProject?.source, modrinthVersions, cfFiles]);

  const hasVersions = selectedProject?.source === 'modrinth' ? modrinthVersions.length > 0 : cfFiles.length > 0;
  const hasSelection = selectedProject?.source === 'modrinth' ? !!selectedModrinthVersion : !!selectedCfFile;

  const selectedFileInfo = useMemo(() => {
    if (selectedProject?.source === 'modrinth' && selectedModrinthVersion) {
      const file = getPrimaryFile(selectedModrinthVersion);
      return file ? { filename: file.filename, size: formatSize(file.size) } : null;
    }
    if (selectedCfFile) {
      return { filename: selectedCfFile.fileName, size: cfFormatSize(selectedCfFile.fileLength) };
    }
    return null;
  }, [selectedProject?.source, selectedModrinthVersion, selectedCfFile]);

  const sourceOptions = [
    { value: 'modrinth', label: 'Modrinth' },
    ...(cfAvailable ? [{ value: 'curseforge', label: 'CurseForge' }] : []),
  ];

  // External link for the project
  const getExternalUrl = (project: DisplayProject) => {
    if (project.source === 'modrinth' && project.modrinthProject) {
      const type = contentType === 'datapacks' ? 'datapack' : contentType === 'plugins' ? 'plugin' : 'mod';
      return `https://modrinth.com/${type}/${project.modrinthProject.slug}`;
    }
    if (project.source === 'curseforge' && project.curseforgeProject) {
      return `https://www.curseforge.com/minecraft/mc-mods/${project.curseforgeProject.slug}`;
    }
    return null;
  };

  // NOTE: dangerouslySetInnerHTML usage below is intentional — content comes from
  // Modrinth/CurseForge project descriptions (trusted API sources), not user input.

  return (
    <div className='ci-browse'>
      {/* Search bar */}
      <div className='ci-search-bar'>
        <TextInput
          placeholder={`Search ${contentType}...`}
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

      {/* Detection info */}
      {(detection.loader !== 'unknown' || detection.mcVersion) && (
        <Group gap='xs' mt='xs' mb='sm'>
          {detection.loader !== 'unknown' && (
            <Badge variant='light' color='violet' size='sm'>{detection.loader}</Badge>
          )}
          {detection.mcVersion && (
            <Badge variant='light' color='gray' size='sm'>{detection.mcVersion}</Badge>
          )}
          <Text size='xs' c='dimmed'>Results filtered for your server</Text>
        </Group>
      )}

      {/* Card grid */}
      {loading && results.length === 0 ? (
        <div className='ci-center'><Loader color='violet' size='lg' /></div>
      ) : results.length === 0 ? (
        <Text c='dimmed' ta='center' mt='xl'>
          {query ? 'No results found. Try a different search.' : `No ${contentType} found.`}
        </Text>
      ) : (
        <>
          <div className='ci-results-grid'>
            {results.map((project) => (
              <Card
                key={`${project.source}-${project.id}`}
                hoverable
                p='md'
                className='ci-project-card'
                onClick={() => openDetail(project)}
              >
                <div className='ci-card-header'>
                  {project.iconUrl ? (
                    <img src={project.iconUrl} alt='' className='ci-project-icon' />
                  ) : (
                    <div className='ci-project-icon ci-project-icon--placeholder' />
                  )}
                  <div className='ci-card-title'>
                    <Text fw={600} size='sm' lineClamp={1}>{project.title}</Text>
                    <Text size='xs' c='dimmed'>by {project.author}</Text>
                  </div>
                </div>
                <div className='ci-card-body'>
                  <Text size='xs' c='dimmed' lineClamp={3}>{project.description}</Text>
                </div>
                <div className='ci-card-footer'>
                  <Text size='xs' c='dimmed'>
                    {(project.source === 'curseforge' ? cfFormatDownloads : formatDownloads)(project.downloads)} downloads
                  </Text>
                  <Badge variant='light' color={project.source === 'curseforge' ? 'orange' : 'green'} size='xs'>
                    {project.source === 'curseforge' ? 'CurseForge' : 'Modrinth'}
                  </Badge>
                </div>
              </Card>
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

      {/* Detail / Install Modal */}
      <Modal
        opened={!!selectedProject}
        onClose={() => setSelectedProject(null)}
        size='80%'
        title={null}
        padding='lg'
        classNames={{ header: 'ci-modal-header', body: 'ci-modal-body' }}
      >
        {selectedProject && (
          <Stack gap='md'>
            {/* Header row: icon + meta left, version + install right */}
            <div className='ci-detail-top'>
              <div className='ci-detail-top-left'>
                {selectedProject.iconUrl ? (
                  <img src={selectedProject.iconUrl} alt='' className='ci-detail-icon' />
                ) : (
                  <div className='ci-detail-icon ci-detail-icon--placeholder' />
                )}
                <div className='ci-detail-meta'>
                  <Group gap='xs' align='center'>
                    <Text fw={700} size='lg'>{selectedProject.title}</Text>
                    <Badge variant='light' color={selectedProject.source === 'curseforge' ? 'orange' : 'green'} size='xs'>
                      {selectedProject.source === 'curseforge' ? 'CurseForge' : 'Modrinth'}
                    </Badge>
                    {(() => {
                      const url = getExternalUrl(selectedProject);
                      return url ? (
                        <Button
                          size='compact-xs' variant='subtle' component='a'
                          href={url} target='_blank'
                          leftSection={<FontAwesomeIcon icon={faExternalLink} />}
                          onClick={(e: React.MouseEvent) => e.stopPropagation()}
                        >
                          View
                        </Button>
                      ) : null;
                    })()}
                  </Group>
                  <Group gap='xs'>
                    <Text size='xs' c='dimmed'>by {selectedProject.author}</Text>
                    <Text size='xs' c='dimmed'>&middot;</Text>
                    <Text size='xs' c='dimmed'>
                      {(selectedProject.source === 'curseforge' ? cfFormatDownloads : formatDownloads)(selectedProject.downloads)} downloads
                    </Text>
                    {selectedFileInfo && (
                      <>
                        <Text size='xs' c='dimmed'>&middot;</Text>
                        <Text size='xs' c='dimmed'>{selectedFileInfo.size}</Text>
                      </>
                    )}
                    {selectedProject.source === 'modrinth' && selectedModrinthVersion && (
                      <>
                        <Text size='xs' c='dimmed'>&middot;</Text>
                        <Text size='xs' c='dimmed'>{timeAgo(selectedModrinthVersion.date_published)}</Text>
                        {selectedModrinthVersion.version_type !== 'release' && (
                          <Badge color={selectedModrinthVersion.version_type === 'beta' ? 'yellow' : 'red'} variant='light' size='xs'>
                            {selectedModrinthVersion.version_type}
                          </Badge>
                        )}
                      </>
                    )}
                    {selectedProject.source === 'curseforge' && selectedCfFile && selectedCfFile.releaseType !== 1 && (
                      <Badge color={selectedCfFile.releaseType === 2 ? 'yellow' : 'red'} variant='light' size='xs'>
                        {releaseTypeLabel(selectedCfFile.releaseType)}
                      </Badge>
                    )}
                  </Group>
                </div>
              </div>

              <div className='ci-detail-top-right'>
                {versionsLoading ? (
                  <Loader color='violet' size='xs' />
                ) : !hasVersions ? (
                  <Text size='xs' c='dimmed'>No versions</Text>
                ) : (
                  <>
                    <Select
                      placeholder='Version...'
                      data={versionOptions}
                      value={
                        selectedProject.source === 'modrinth'
                          ? (selectedModrinthVersion?.id ?? null)
                          : (selectedCfFile ? String(selectedCfFile.id) : null)
                      }
                      onChange={(val) => {
                        if (selectedProject.source === 'modrinth') {
                          setSelectedModrinthVersion(modrinthVersions.find((v) => v.id === val) ?? null);
                        } else {
                          setSelectedCfFile(cfFiles.find((f) => String(f.id) === val) ?? null);
                        }
                      }}
                      searchable
                      size='sm'
                      w={220}
                    />
                    {installState === 'done' ? (
                      <Button
                        onClick={() => setSelectedProject(null)}
                        color='green'
                        size='sm'
                        leftSection={<FontAwesomeIcon icon={faCheck} />}
                      >
                        Done
                      </Button>
                    ) : (
                      <Button
                        onClick={doInstall}
                        loading={installing && installState !== 'error'}
                        disabled={
                          !hasSelection || !hasVersions
                          || (selectedProject.source === 'curseforge' && selectedCfFile && !selectedCfFile.downloadUrl)
                        }
                        color={installState === 'error' ? 'red' : existingFile ? 'yellow' : undefined}
                        size='sm'
                        leftSection={
                          <FontAwesomeIcon icon={installState === 'error' ? faRefresh : existingFile ? faArrowUp : faArrowDown} />
                        }
                      >
                        {installState === 'idle' && (existingFile ? 'Update' : 'Install')}
                        {installState === 'downloading' && (existingFile ? 'Updating...' : 'Installing...')}
                        {installState === 'error' && 'Retry'}
                      </Button>
                    )}
                  </>
                )}
              </div>
            </div>

            {existingFile && (
              <Alert color='yellow' variant='light'>
                Already installed as <strong>{existingFile}</strong>. Installing a new version will replace it.
              </Alert>
            )}

            {selectedProject.source === 'curseforge' && selectedCfFile && !selectedCfFile.downloadUrl && (
              <Alert color='red' variant='light'>
                This mod does not allow third-party downloads.
              </Alert>
            )}

            {/* Description — content from Modrinth/CurseForge APIs (trusted sources) */}
            {detailLoading ? (
              <div className='ci-center'><Loader color='violet' size='sm' /></div>
            ) : detailBody ? (
              <div
                className='ci-detail-body'
                dangerouslySetInnerHTML={{
                  __html: selectedProject.source === 'curseforge'
                    ? detailBody
                    : convertMarkdown(detailBody),
                }}
              />
            ) : (
              <Text size='sm' c='dimmed'>{selectedProject.description}</Text>
            )}
          </Stack>
        )}
      </Modal>
    </div>
  );
}

function convertMarkdown(md: string): string {
  return marked.parse(md, { async: false, breaks: true }) as string;
}
