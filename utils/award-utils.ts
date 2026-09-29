import fs from 'fs/promises';
import path from 'path';

export async function renderAwardPage(awardId: string): Promise<string> {
  const awardData = JSON.parse(
    await fs.readFile(path.join(__dirname, `../awards/cad/${awardId}.json`), 'utf-8')
  );
  const template = await fs.readFile(path.join(__dirname, `../pages/awards/${awardId}.md`), 'utf-8');
  return template.replace(/{{amount}}/g, awardData.amount.toString());
}

export async function validateMetrics(): Promise<any> {
  return JSON.parse(
    await fs.readFile(path.join(__dirname, '../metrics/homepage.yml'), 'utf-8')
  );
}

export async function checkDuplicateProtection(awardId: string): Promise<boolean> {
  const awardsDir = await fs.readdir(path.join(__dirname, '../awards/cad'));
  return awardsDir.filter(id => id !== `${awardId}.json`).length > 0;
}