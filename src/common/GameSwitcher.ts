/** 两个游戏选择入口共用常用游戏及用户排序，不修改原始列表。 */
export const orderedFavoriteGames = <T extends { name: string; showSidebar: boolean }>(
    games: readonly T[], order: readonly string[],
): T[] => {
    const indices = new Map(order.map((name, index) => [name, index]));
    return games.filter(game => game.showSidebar).sort((a, b) =>
        (indices.get(a.name) ?? Number.MAX_SAFE_INTEGER) - (indices.get(b.name) ?? Number.MAX_SAFE_INTEGER));
};
