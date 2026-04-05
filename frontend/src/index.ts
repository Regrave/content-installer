import { faPuzzlePiece } from '@fortawesome/free-solid-svg-icons';
import { Extension, ExtensionContext } from 'shared';
import AdminConfigPage from './AdminConfigPage.tsx';
import ContentInstallerPage from './ContentInstallerPage.tsx';

class ContentInstaller extends Extension {
  public cardConfigurationPage = AdminConfigPage;

  public initialize(ctx: ExtensionContext): void {
    ctx.extensionRegistry.routes.addServerRoute({
      name: 'Content',
      icon: faPuzzlePiece,
      path: '/content',
      element: ContentInstallerPage,
      permission: 'files.create',
    });
  }
}

export default new ContentInstaller();
