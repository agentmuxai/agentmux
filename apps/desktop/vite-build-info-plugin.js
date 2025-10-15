import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export default function buildInfoPlugin() {
  return {
    name: 'build-info',
    transform(code, id) {
      if (id.includes('App.tsx')) {
        // Read version from package.json
        const packageJson = JSON.parse(
          readFileSync(join(__dirname, 'package.json'), 'utf-8')
        );
        const version = packageJson.version;

        // Generate build timestamp in PST
        const now = new Date();
        const pstTime = new Intl.DateTimeFormat('en-US', {
          timeZone: 'America/Los_Angeles',
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
          hour: 'numeric',
          minute: '2-digit',
          hour12: true
        }).format(now);

        // Replace placeholders (both quoted and unquoted)
        code = code.replace(/'__VERSION__'/g, `'${version}'`);
        code = code.replace(/__VERSION__/g, `'${version}'`);
        code = code.replace(/'__BUILD_TIME__'/g, `'${pstTime}'`);
        code = code.replace(/__BUILD_TIME__/g, `'${pstTime}'`);

        return { code };
      }
    }
  };
}
