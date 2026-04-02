import { faArrowDown, faCheck, faExternalLink, faRefresh, faSearch } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Alert,
  Badge,
  Group,
  Loader,
  Modal,
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
import { LOADER_TO_MODRINTH } from './detect.ts';
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

interface BrowseTabProps {
  detection: ServerDetection;
  contentType: 'plugins' | 'mods' | 'datapacks';
  installDir: string;
  onInstalled?: () => void;
}

type InstallState = 'idle' | 'downloading' | 'done' | 'error';

export default function BrowseTab({ detection, contentType, installDir, onInstalled }: BrowseTabProps) {
  const { addToast } = useToast();
  const { server } = useServerStore();

  // Search state
  const [query, setQuery] = useState('');
  const [sortBy, setSortBy] = useState<SearchIndex>('relevance');
  const [results, setResults] = useState<ModrinthProject[]>([]);
  const [totalHits, setTotalHits] = useState(0);
  const [page, setPage] = useState(0);
  const [loading, setLoading] = useState(false);
  const searchTimer = useRef<ReturnType<typeof setTimeout>>();

  // Install modal state
  const [selectedProject, setSelectedProject] = useState<ModrinthProject | null>(null);
  const [versions, setVersions] = useState<ModrinthVersion[]>([]);
  const [selectedVersion, setSelectedVersion] = useState<ModrinthVersion | null>(null);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [installState, setInstallState] = useState<InstallState>('idle');
  const [installing, setInstalling] = useState(false);

  const projectType = contentType === 'datapacks' ? 'datapack' as const
    : contentType === 'plugins' ? 'plugin' as const : 'mod' as const;
  const loaders = contentType === 'datapacks' ? ['datapack'] : (LOADER_TO_MODRINTH[detection.loader] ?? []);

  // Search function
  const doSearch = useCallback(async (q: string, sort: SearchIndex, offset: number) => {
    setLoading(true);
    try {
      const res = await searchProjects({
        query: q || undefined,
        projectType,
        loaders: loaders.length > 0 ? loaders : undefined,
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
      addToast(`Search failed: ${err instanceof Error ? err.message : 'unknown error'}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [projectType, loaders, detection.mcVersion]);

  // Initial search + debounced search on query change
  useEffect(() => {
    setPage(0);
    if (searchTimer.current) clearTimeout(searchTimer.current);
    searchTimer.current = setTimeout(() => doSearch(query, sortBy, 0), 300);
    return () => { if (searchTimer.current) clearTimeout(searchTimer.current); };
  }, [query, sortBy, doSearch]);

  const loadMore = () => {
    const newOffset = results.length;
    setPage(newOffset);
    doSearch(query, sortBy, newOffset);
  };

  // Open install modal for a project
  const openInstall = useCallback(async (project: ModrinthProject) => {
    setSelectedProject(project);
    setInstallState('idle');
    setSelectedVersion(null);
    setVersionsLoading(true);

    try {
      const vers = await getProjectVersions(project.project_id, {
        loaders: loaders.length > 0 ? loaders : undefined,
        gameVersions: detection.mcVersion ? [detection.mcVersion] : undefined,
      });
      setVersions(vers);
      // Auto-select first featured or first version
      const featured = vers.find((v) => v.featured) ?? vers[0];
      if (featured) setSelectedVersion(featured);
    } catch (err) {
      addToast(`Failed to load versions: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setVersionsLoading(false);
    }
  }, [loaders, detection.mcVersion]);

  // Install selected version
  const doInstall = useCallback(async () => {
    if (!selectedVersion) return;
    const file = getPrimaryFile(selectedVersion);
    if (!file) {
      addToast('No downloadable file found for this version.', 'error');
      return;
    }

    setInstalling(true);
    setInstallState('downloading');

    try {
      const params = new URLSearchParams({
        url: file.url,
        filename: file.filename,
        directory: installDir,
      });

      const res = await fetch(
        `/api/client/servers/${server.uuid}/content-installer/install?${params}`,
        { method: 'POST' },
      );
      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `Install failed: ${res.status}`);
      }

      // Poll status
      const statusUrl = `/api/client/servers/${server.uuid}/content-installer/install/status`;
      for (let i = 0; i < 60; i++) {
        await new Promise((r) => setTimeout(r, 1500));
        const statusRes = await fetch(statusUrl);
        if (!statusRes.ok) break;
        const status = await statusRes.json();
        if (status.state === 'done') break;
      }

      setInstallState('done');
      addToast(`Installed ${selectedProject?.title ?? file.filename}!`, 'success');
      onInstalled?.();
    } catch (err) {
      setInstallState('error');
      addToast(`Install failed: ${err instanceof Error ? err.message : 'unknown'}`, 'error');
    } finally {
      setInstalling(false);
    }
  }, [selectedVersion, selectedProject, contentType, server.uuid]);

  // Version options for selector
  const versionOptions = useMemo(() =>
    versions.map((v) => ({
      value: v.id,
      label: `${v.version_number} (${v.game_versions.slice(0, 3).join(', ')}${v.game_versions.length > 3 ? '...' : ''})`,
    })),
    [versions],
  );

  const selectedFile = selectedVersion ? getPrimaryFile(selectedVersion) : null;

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

      {/* Detection info */}
      {(detection.loader !== 'unknown' || detection.mcVersion) && (
        <Group gap='xs' mt='xs' mb='sm'>
          {detection.loader !== 'unknown' && (
            <Badge variant='light' color='violet' size='sm'>
              {detection.loader}
            </Badge>
          )}
          {detection.mcVersion && (
            <Badge variant='light' color='gray' size='sm'>
              {detection.mcVersion}
            </Badge>
          )}
          <Text size='xs' c='dimmed'>
            Results filtered for your server
          </Text>
        </Group>
      )}

      {/* Results grid */}
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

          {/* Load more */}
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
        onClose={() => setSelectedProject(null)}
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
      >
        {selectedProject && (
          <Stack gap='md'>
            <Text size='sm' c='dimmed'>{selectedProject.description}</Text>

            {versionsLoading ? (
              <div className='ci-center'><Loader color='violet' size='sm' /></div>
            ) : versions.length === 0 ? (
              <Alert color='yellow' variant='light'>
                No compatible versions found for your server
                {detection.mcVersion ? ` (${detection.mcVersion})` : ''}.
              </Alert>
            ) : (
              <>
                <Select
                  label='Version'
                  placeholder='Select version...'
                  data={versionOptions}
                  value={selectedVersion?.id ?? null}
                  onChange={(val) => {
                    const v = versions.find((ver) => ver.id === val);
                    setSelectedVersion(v ?? null);
                  }}
                  searchable
                />

                {selectedVersion && (
                  <div className='ci-version-info'>
                    {selectedFile && (
                      <Text size='xs' c='dimmed'>
                        {selectedFile.filename} &middot; {formatSize(selectedFile.size)}
                      </Text>
                    )}
                    <Group gap='xs' mt={4}>
                      {selectedVersion.version_type !== 'release' && (
                        <Badge
                          color={selectedVersion.version_type === 'beta' ? 'yellow' : 'red'}
                          variant='light'
                          size='xs'
                        >
                          {selectedVersion.version_type}
                        </Badge>
                      )}
                      <Text size='xs' c='dimmed'>
                        {formatDownloads(selectedVersion.downloads)} downloads &middot; {timeAgo(selectedVersion.date_published)}
                      </Text>
                    </Group>
                    {selectedVersion.loaders.length > 0 && (
                      <Group gap={4} mt={4}>
                        {selectedVersion.loaders.map((l) => (
                          <Badge key={l} variant='light' color='gray' size='xs'>{l}</Badge>
                        ))}
                      </Group>
                    )}
                  </div>
                )}
              </>
            )}

            <Group justify='flex-end' mt='sm'>
              <Button variant='subtle' onClick={() => setSelectedProject(null)}>
                Cancel
              </Button>
              <Button
                onClick={installState === 'error' ? doInstall : doInstall}
                loading={installing && installState !== 'done' && installState !== 'error'}
                disabled={!selectedVersion || !selectedFile || versions.length === 0}
                color={installState === 'done' ? 'green' : installState === 'error' ? 'red' : undefined}
                leftSection={
                  <FontAwesomeIcon
                    icon={installState === 'done' ? faCheck : installState === 'error' ? faRefresh : faArrowDown}
                  />
                }
              >
                {installState === 'idle' && 'Install'}
                {installState === 'downloading' && 'Installing...'}
                {installState === 'done' && 'Installed!'}
                {installState === 'error' && 'Retry'}
              </Button>
            </Group>
          </Stack>
        )}
      </Modal>
    </div>
  );
}
