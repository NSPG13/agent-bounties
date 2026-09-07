// Keep idb-keyval's database, schema and transaction callback contract. Only
// acquiring a transaction may be retried; an operation that started is never
// replayed, including when its callback rejects with InvalidStateError.
export function createReconnectingStore(dbName, storeName) {
  let connection;
  function open() {
    if (!connection) {
      const pending = new Promise((resolve, reject) => {
        const request = indexedDB.open(dbName);
        request.onupgradeneeded = () => request.result.createObjectStore(storeName);
        request.onerror = () => reject(request.error);
        request.onsuccess = () => {
          const db = request.result;
          const invalidate = () => {
            if (connection === pending) connection = undefined;
            db.close();
          };
          db.onclose = invalidate;
          db.onversionchange = invalidate;
          resolve(db);
        };
      });
      connection = pending;
      // A failed open must not poison future connection attempts.
      void pending.catch(() => { if (connection === pending) connection = undefined; });
    }
    return connection;
  }
  return async (txMode, callback) => {
    for (let attempt = 0; ; attempt++) {
      const pending = open();
      const db = await pending;
      let transaction;
      try {
        transaction = db.transaction(storeName, txMode);
      } catch (error) {
        if (error?.name !== "InvalidStateError") throw error;
        if (connection === pending) connection = undefined;
        db.close();
        if (attempt === 0) continue;
        throw error;
      }
      return callback(transaction.objectStore(storeName));
    }
  };
}
