import { faCheck, faKey } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Alert, Group, Loader, Stack, Text, TextInput, Title } from '@mantine/core';
import { useEffect, useState } from 'react';
import Button from '@/elements/Button.tsx';

export default function AdminConfigPage() {
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [configured, setConfigured] = useState(false);
  const [maskedKey, setMaskedKey] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    fetch('/api/admin/content-installer/settings')
      .then((r) => r.json())
      .then((data) => {
        setConfigured(data.curseforge_configured);
        setMaskedKey(data.curseforge_api_key_masked ?? '');
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const handleSave = async () => {
    setSaving(true);
    setSaved(false);
    try {
      const res = await fetch('/api/admin/content-installer/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ curseforge_api_key: apiKey }),
      });
      if (res.ok) {
        setConfigured(true);
        const key = apiKey;
        setMaskedKey(key.length > 8 ? `${key.slice(0, 4)}...${key.slice(-4)}` : '*'.repeat(key.length));
        setApiKey('');
        setSaved(true);
        setTimeout(() => setSaved(false), 3000);
      }
    } catch {}
    setSaving(false);
  };

  const handleClear = async () => {
    setSaving(true);
    try {
      const res = await fetch('/api/admin/content-installer/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ curseforge_api_key: '' }),
      });
      if (res.ok) {
        setConfigured(false);
        setMaskedKey('');
        setApiKey('');
      }
    } catch {}
    setSaving(false);
  };

  if (loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: 48 }}>
        <Loader color='violet' />
      </div>
    );
  }

  return (
    <Stack gap='lg' mt='md'>
      <div>
        <Title order={4}>
          <FontAwesomeIcon icon={faKey} style={{ marginRight: 8 }} />
          CurseForge Integration
        </Title>
        <Text size='sm' c='dimmed' mt={4}>
          Enter your CurseForge API key to enable searching and installing content from CurseForge.
          Get a key from the CurseForge developer console.
        </Text>
      </div>

      {configured ? (
        <Alert color='green' variant='light'>
          CurseForge is configured. API key: <strong>{maskedKey}</strong>
        </Alert>
      ) : (
        <Alert color='yellow' variant='light'>
          CurseForge is not configured. Users will only be able to browse Modrinth.
        </Alert>
      )}

      <div>
        <TextInput
          label='CurseForge API Key'
          placeholder={configured ? 'Enter a new key to replace the current one' : 'Enter your CurseForge API key'}
          value={apiKey}
          onChange={(e) => setApiKey(e.currentTarget.value)}
          type='password'
        />
      </div>

      <Group>
        <Button
          onClick={handleSave}
          loading={saving}
          disabled={!apiKey.trim()}
          color={saved ? 'green' : undefined}
          leftSection={saved ? <FontAwesomeIcon icon={faCheck} /> : undefined}
        >
          {saved ? 'Saved!' : 'Save API Key'}
        </Button>
        {configured && (
          <Button variant='subtle' color='red' onClick={handleClear} loading={saving}>
            Remove Key
          </Button>
        )}
      </Group>
    </Stack>
  );
}
