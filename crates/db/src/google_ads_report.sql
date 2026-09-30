WITH names AS (
  SELECT DISTINCT campaign FROM google_ads_clicks
  UNION SELECT DISTINCT campaign FROM google_ads_campaign_costs
), outcomes AS (
  SELECT * FROM google_ads_attributed_outcomes WHERE event_at >= $1 AND event_at < $2
), costs AS (
  SELECT campaign,sum(cost_micros)::double precision/1000000 AS cost,
    sum(clicks)::bigint AS clicks,count(*) AS cost_days
  FROM google_ads_campaign_costs
  WHERE day>=($1 AT TIME ZONE 'Etc/GMT+6')::date
    AND day<($2 AT TIME ZONE 'Etc/GMT+6')::date GROUP BY campaign
)
SELECT n.campaign,
 (SELECT count(DISTINCT creator_wallet) FROM outcomes e WHERE e.first_touch_campaign=n.campaign AND e.first_touch_rail='google-ads' AND e.outcome='external_bounty_funded') AS posters,
 (SELECT count(*) FROM outcomes e WHERE e.first_touch_campaign=n.campaign AND e.first_touch_rail='google-ads' AND e.outcome='external_bounty_funded') AS funded,
 (SELECT count(*) FROM outcomes e WHERE e.first_touch_campaign=n.campaign AND e.first_touch_rail='google-ads' AND e.outcome='verified_bounty_paid') AS paid,
 (SELECT count(*) FROM outcomes e WHERE e.campaign=n.campaign AND (e.first_touch_rail<>'google-ads' OR e.first_touch_campaign<>n.campaign) AND e.outcome='external_bounty_funded') AS assisted,
 c.cost,c.clicks,COALESCE(c.cost_days,0) AS cost_days
FROM names n LEFT JOIN costs c USING(campaign) ORDER BY n.campaign;
